mod common;
use common::{fixture_lines, Worker};
use serde_json::{json, Value};
use solver_worker::extract::NodeSite;
use solver_worker::job::{Checkpoint, Hooks, Op};
use solver_worker::protocol::{executor_loop_with, handle_line, Proto, Shared, WorkerState};
use solver_worker::solve_loop::LoopSite;
use solver_worker::writer::Out;
use std::sync::mpsc::{channel, sync_channel, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn with_id(line: &str, id: &str) -> String { edit(line, |v| v["id"] = json!(id)) }
fn cancel(id: &str, target: &str) -> String { json!({"type": "cancel", "id": id, "target": target}).to_string() }
fn ack_of(w: &Worker, id: &str) -> Value { w.recv_until(Duration::from_secs(5), |m| m["type"] == "ack" && m["id"] == id).unwrap_or_else(|| panic!("no ack for {id}")) }
fn result_of(w: &Worker, id: &str, t: Duration) -> Value { w.recv_until(t, |m| m["type"] == "result" && m["id"] == id).unwrap_or_else(|| panic!("no result for {id}")) }
const S: Duration = Duration::from_secs(1);

/// A message as one comparable line: `ack <id> <status>[ <reason>][ replaced=<b>]`, `result <id> <status>[ <code>]` or
/// `progress <id> <stage>`.
fn brief(v: &Value) -> String {
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    match s("type").as_str() {
        "ack" => {
            let mut b = format!("ack {} {}", s("id"), s("status"));
            if let Some(r) = v["reason"].as_str() { b += &format!(" {r}"); }
            if let Some(r) = v["replaced"].as_bool() { b += &format!(" replaced={r}"); }
            b
        }
        "result" => {
            let mut b = format!("result {} {}", s("id"), s("status"));
            if let Some(c) = v["error"]["code"].as_str() { b += &format!(" {c}"); }
            b
        }
        "progress" => format!("progress {} {}", s("id"), s("stage")),
        other => panic!("unexpected message type {other:?}: {v}"),
    }
}
fn briefs(vs: &[Value]) -> Vec<String> { vs.iter().map(brief).collect() }
/// The messages that are not progress reports, in order.
fn protocol_only(vs: &[Value]) -> Vec<String> { briefs(vs).into_iter().filter(|m| !m.starts_with("progress ")).collect() }

#[test]
fn lock_staging_rejections() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let river = fixture_lines("river_two_combo");
    let lock = fixture_lines("lock_river");
    // a row summing to 0.4 and a row [-0.1, 1.1] are rejected; an all-zero row is the free-combo sentinel
    w.send(&edit(&with_id(&lock[0], "7"), |v| v["locks"][0]["probs"][0] = json!([0.1, 0.3])));
    assert_eq!(ack_of(&w, "7")["status"], "rejected");
    w.send(&edit(&with_id(&lock[0], "8"), |v| v["locks"][0]["probs"][0] = json!([-0.1, 1.1])));
    assert_eq!(ack_of(&w, "8")["status"], "rejected");
    w.send(&with_id(&lock[0], "9"));
    let a = ack_of(&w, "9");
    assert_eq!((a["status"].as_str(), a["replaced"].as_bool()), (Some("staged"), Some(false)));
    w.send(&with_id(&lock[0], "10"));
    assert_eq!(ack_of(&w, "10")["replaced"], true);
    // a staged lock belonging to another spot fails the next solve with lock_mismatch and is discarded
    w.send(&with_id(&river[0], "14"));
    assert_eq!(ack_of(&w, "14")["status"], "accepted");
    let r = result_of(&w, "14", 5 * S);
    assert_eq!((r["status"].as_str(), r["error"]["code"].as_str()), (Some("error"), Some("lock_mismatch")));
    w.send(&with_id(&river[0], "15"));
    assert_eq!(result_of(&w, "15", 5 * S)["status"], "ok");
    w.close_stdin();
    assert_eq!(w.wait_exit(2 * S), Some(0));
}

/// Fix round 1 (review P2.T14-I1, the reviewer's replay on the fixture): a `solve` the wire codec itself rejects (here
/// `rake_rate: 1.0`, outside the codec's half-open domain) still identifies itself as a `solve`, so it takes the staged
/// set with it like a solve rejected at precheck or admission: the next, valid solve of the staged spot runs unlocked.
#[test]
fn a_solve_the_wire_codec_rejects_discards_the_staged_set() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let lock = fixture_lines("lock_river");   // lock 47, its solve 51, shutdown 52
    w.send(&lock[0]);
    let a = ack_of(&w, "47");
    assert_eq!((a["status"].as_str(), a["replaced"].as_bool()), (Some("staged"), Some(false)));
    w.send(&edit(&with_id(&lock[1], "50"), |v| v["rake_rate"] = json!(1.0)));
    let a = ack_of(&w, "50");
    assert_eq!(a["status"], "rejected");
    assert!(a["reason"].as_str().unwrap().starts_with("invalid message: rake_rate"), "{a}");
    w.send(&lock[1]);
    assert_eq!(ack_of(&w, "51")["status"], "accepted");
    let r = result_of(&w, "51", 5 * S);
    assert_eq!(r["status"], "ok");
    assert_eq!(r["solution"]["locks_applied"], 0, "the rejected solve took the staged set");
    w.send(&lock[2]);
    assert_eq!(ack_of(&w, "52")["status"], "accepted");
    assert_eq!(w.wait_exit(2 * S), Some(0));
}

// ---- In process: the real control handlers and executor, the job held by a deterministic checkpoint barrier ----

/// A liveness bound on every wait that must not race the machine's own speed, never a placement: the barrier tests
/// below decide where an in-process job is by site, not by this bound; `lock_lifecycle`'s spawned-worker shutdown
/// exit (a concurrent gate's other builds and test binaries can starve the child process well past a short fixed
/// timeout) is awaited the same way, generous headroom over every observed run.
const LIVENESS: Duration = Duration::from_secs(60);

/// A point of a job as the executor's hooks report it (`job::Hooks`), on the job's own thread, immediately before the
/// cancel poll or the work it names: a job checkpoint, an operation starting or completing, a site of the §7 loop (a
/// cancel poll, a solve step or a measurement, with the iterations completed), or a site of the export (a cancel poll
/// or a node's extraction, with the nodes extracted). Every unit of work a job does is one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Site { At(Checkpoint), Enter(Op), Leave(Op), Loop(LoopSite), Node(NodeSite) }

/// The barrier's state: every site the executor's jobs passed, in order; the armed site; whether a job is held there.
#[derive(Default)]
struct Gate { passed: Vec<Site>, armed: Option<Site>, held: bool }

/// The deterministic checkpoint barrier. Installed as the executor's hooks, it records every site and holds the job at
/// the armed site (once) until the test releases it: placement is by site, never by time. Hooks never run under the
/// protocol lock, so control goes on answering while a job is held.
#[derive(Clone, Default)]
struct Barrier(Arc<(Mutex<Gate>, Condvar)>);
impl Barrier {
    fn arm(&self, at: Site) {
        let mut g = self.0.0.lock().unwrap();
        assert!(g.armed.is_none() && !g.held, "one hold at a time");
        g.armed = Some(at);
    }
    /// Waits until a job is held at the armed site, and returns the sites passed so far, the held one last.
    fn wait_held(&self) -> Vec<Site> {
        let (m, cv) = &*self.0;
        let (g, wait) = cv.wait_timeout_while(m.lock().unwrap(), LIVENESS, |g| !g.held).unwrap();
        assert!(!wait.timed_out(), "no job reached {:?}", g.armed);
        g.passed.clone()
    }
    fn release(&self) {
        let (m, cv) = &*self.0;
        m.lock().unwrap().held = false;
        cv.notify_all();
    }
    fn passed(&self) -> Vec<Site> { self.0.0.lock().unwrap().passed.clone() }
    fn pass(&self, at: Site) {
        let (m, cv) = &*self.0;
        let mut g = m.lock().unwrap();
        g.passed.push(at);
        if g.armed == Some(at) {
            g.armed = None;
            g.held = true;
            cv.notify_all();
            while g.held { g = cv.wait(g).unwrap(); }
        }
    }
}
impl Hooks for Barrier {
    fn checkpoint(&mut self, at: Checkpoint) { self.pass(Site::At(at)) }
    fn enter(&mut self, op: Op) { self.pass(Site::Enter(op)) }
    fn leave(&mut self, op: Op) { self.pass(Site::Leave(op)) }
    fn loop_site(&mut self, at: LoopSite) { self.pass(Site::Loop(at)) }
    fn node_site(&mut self, at: NodeSite) { self.pass(Site::Node(at)) }
}

/// The worker in process, wired as `main` wires it (the shared state, the executor thread on the jobs channel, `out`
/// for stdout) with the barrier as the executor's hooks. The test is control (`send`) and reads `out` in the writer's
/// place; every message is kept, in order, none dropped (`seen`).
struct Harness { shared: Arc<Shared>, out: Receiver<Out>, barrier: Barrier, seen: Vec<Value> }
impl Harness {
    fn new() -> Harness {
        let (out_tx, out) = sync_channel::<Out>(4096);
        let (jobs_tx, jobs) = channel();
        let shared = Arc::new(Shared { proto: Mutex::new(Proto::new()), out: out_tx, jobs: jobs_tx });
        let barrier = Barrier::default();
        let (exec, mut hooks) = (Arc::clone(&shared), barrier.clone());
        std::thread::spawn(move || executor_loop_with(exec, jobs, &mut hooks));
        Harness { shared, out, barrier, seen: Vec::new() }
    }
    fn keep(&mut self, o: Out) -> Value {
        let v = match o { Out::Msg(m) => serde_json::to_value(&m).unwrap(), Out::Exit(c) => panic!("unexpected Exit({c})") };
        self.seen.push(v.clone());
        v
    }
    /// One line through control, exactly as `main` hands it over; everything queued up to its answer, which is
    /// queued before `handle_line` returns and so comes last.
    fn send(&mut self, line: &str) -> Vec<String> {
        handle_line(&self.shared, line);
        let queued: Vec<Out> = self.out.try_iter().collect();
        queued.into_iter().map(|o| brief(&self.keep(o))).collect()
    }
    /// Everything queued up to and including job `id`'s terminal.
    fn until_result(&mut self, id: &str) -> Vec<Value> {
        let end = Instant::now() + LIVENESS;
        let mut got = Vec::new();
        loop {
            let o = self.out.recv_timeout(end.saturating_duration_since(Instant::now())).unwrap_or_else(|e| panic!("no result for {id} after {got:?}: {e:?}"));
            let v = self.keep(o);
            let done = v["type"] == "result" && v["id"] == id;
            got.push(v);
            if done { return got; }
        }
    }
    fn state(&self) -> WorkerState { self.shared.proto.lock().unwrap().state }
    fn staged(&self) -> bool { self.shared.proto.lock().unwrap().staged.is_some() }
}
/// A failed assertion never leaves a job held behind it.
impl Drop for Harness { fn drop(&mut self) { self.barrier.release(); } }

/// §13.2 `cancel_between_iterations` on the flop fixture, deterministic (fix round 1, review P2.T14-I2). The worker runs
/// in process with the real control handlers and executor, and the barrier holds the job at a Building step (the
/// checkpoint after the tree build), at a Solving iteration boundary (the loop's poll after two iterations) and at an
/// Extracting node boundary (the export's poll after one node). Each time, the worker's own state (moved by the job's
/// progress) is that stage; the cancel goes through control and its `accepted` ack is queued while the job is still
/// held; released, the job is answered at that very boundary: the only site it passes afterwards closes the operation
/// it was in (no checkpoint, operation, solve step, measurement or node extraction follows, each of which the hooks
/// would report), and exactly one terminal, `cancelled`, follows the ack with nothing between them; a second cancel
/// is `already_finished`. Separately, completion wins: a job released
/// from its last checkpoint with no cancel pending delivers its solution, and the cancel that comes after it is
/// `already_finished`, queued behind the terminal, never a second terminal. §13.2's latency bounds are measured through
/// the spawned binary (`flop_cancel_fixture_smoke`).
#[test]
fn cancel_between_iterations() {
    let flop = fixture_lines("flop_cancel");
    // the first exploitability measurement reaches the target, so the job goes on to extract without a long solve
    let quick = |id: &str| edit(&with_id(&flop[0], id), |v| v["target_bp"] = json!(u16::MAX));
    let mut h = Harness::new();
    let steps = |sites: &[Site]| sites.iter().filter(|s| matches!(s, Site::Loop(LoopSite::Iteration(_)))).copied().collect::<Vec<_>>();
    let cases = [
        ("Building, after the tree build", with_id(&flop[0], "43"), ["43", "44", "45"], Site::At(Checkpoint::TreeBuilt), WorkerState::Building, vec![]),
        ("Solving, at the boundary after two iterations", with_id(&flop[0], "46"), ["46", "47", "48"], Site::Loop(LoopSite::Boundary(2)), WorkerState::Solving, vec![Site::Leave(Op::Solve)]),
        ("Extracting, at the boundary after one node", quick("49"), ["49", "50", "51"], Site::Node(NodeSite::Poll(1)), WorkerState::Extracting, vec![Site::Leave(Op::Export)]),
    ];
    for (what, solve, [id, cancel_id, late_id], hold, stage, closing) in cases {
        let (from, sites) = (h.seen.len(), h.barrier.passed().len());
        h.barrier.arm(hold);
        assert_eq!(h.send(&solve)[0], format!("ack {id} accepted"), "{what}");
        let before = h.barrier.wait_held()[sites..].to_vec();
        assert_eq!(h.state(), stage, "{what}: the worker's own state while the job is held");
        match hold {
            Site::At(_) => assert_eq!(before, [Site::At(Checkpoint::Start), Site::Enter(Op::TreeBuild), Site::Leave(Op::TreeBuild), hold], "{what}"),
            Site::Loop(_) => assert_eq!(steps(&before), [Site::Loop(LoopSite::Iteration(1)), Site::Loop(LoopSite::Iteration(2))], "{what}: two iterations ran"),
            _ => assert_eq!(before.iter().filter(|s| matches!(s, Site::Node(_))).copied().collect::<Vec<_>>(), [NodeSite::Poll(0), NodeSite::Extract(0), NodeSite::Poll(1)].map(Site::Node), "{what}: one node extracted"),
        }
        let queued = h.send(&cancel(cancel_id, id));
        assert_eq!(queued.last(), Some(&format!("ack {cancel_id} accepted")), "{what}: acked while the job is held: {queued:?}");
        assert!(queued[..queued.len() - 1].iter().all(|m| m.starts_with(&format!("progress {id} "))), "{what}: only the job's progress ahead of it: {queued:?}");
        h.barrier.release();
        assert_eq!(briefs(&h.until_result(id)), [format!("result {id} cancelled")], "{what}: the terminal follows the ack, nothing between");
        let after = h.barrier.passed()[sites + before.len()..].to_vec();
        assert_eq!(after, closing, "{what}: nothing starts past the boundary");
        assert_eq!(h.send(&cancel(late_id, id)), [format!("ack {late_id} already_finished")], "{what}: one terminal");
        assert_eq!(h.state(), WorkerState::Idle, "{what}");
        // the whole exchange, kept in order: the job's progress ends at the held stage and precedes the cancel's ack
        let log = &h.seen[from..];
        assert_eq!(protocol_only(log), [format!("ack {id} accepted"), format!("ack {cancel_id} accepted"), format!("result {id} cancelled"), format!("ack {late_id} already_finished")], "{what}");
        let acked = briefs(log).iter().position(|m| *m == format!("ack {cancel_id} accepted")).unwrap();
        let reports: Vec<String> = briefs(&log[..acked]).into_iter().filter(|m| m.starts_with("progress ")).collect();
        assert_eq!(reports.last(), Some(&format!("progress {id} {}", format!("{stage:?}").to_lowercase())), "{what}: {reports:?}");
        assert!(briefs(&log[acked..]).iter().all(|m| !m.starts_with("progress ")), "{what}: no progress once cancelled");
    }

    // Completion wins: held at its last checkpoint, then released with no cancel pending, the job delivers its solution;
    // the cancel that comes after it is `already_finished`, queued behind the terminal, and no second terminal follows.
    let from = h.seen.len();
    h.barrier.arm(Site::At(Checkpoint::Validated));
    assert_eq!(h.send(&quick("52"))[0], "ack 52 accepted");
    h.barrier.wait_held();
    assert_eq!(h.state(), WorkerState::Extracting);
    h.barrier.release();
    let done = h.until_result("52");
    let r = done.last().unwrap();
    assert_eq!((r["status"].as_str(), r["solution"]["export"].as_str()), (Some("ok"), Some("street")), "the job's own solution");
    assert_eq!(h.send(&cancel("53", "52")), ["ack 53 already_finished"]);
    assert_eq!(h.barrier.passed().last(), Some(&Site::At(Checkpoint::Validated)), "nothing ran after the last checkpoint");
    assert_eq!(protocol_only(&h.seen[from..]), ["ack 52 accepted", "result 52 ok", "ack 53 already_finished"]);
    assert_eq!(h.state(), WorkerState::Idle);
}

/// Every message up to and including the first `pred` accepts, in order, within the liveness bound `t`.
fn collect(w: &Worker, t: Duration, pred: impl Fn(&Value) -> bool) -> Vec<Value> {
    let end = Instant::now() + t;
    let mut got = Vec::new();
    loop {
        let v = w.recv(end.saturating_duration_since(Instant::now())).unwrap_or_else(|| panic!("timed out after {:?}", briefs(&got)));
        let done = pred(&v);
        got.push(v);
        if done { return got; }
    }
}
/// Every message that arrives within `t`: a bounded window for a negative assertion, never a placement.
fn window(w: &Worker, t: Duration) -> Vec<Value> {
    let end = Instant::now() + t;
    let mut got = Vec::new();
    while let Some(v) = w.recv(end.saturating_duration_since(Instant::now())) { got.push(v); }
    got
}
fn result_for(id: &'static str) -> impl Fn(&Value) -> bool { move |m| m["type"] == "result" && m["id"] == id }
fn ack_for(id: &'static str) -> impl Fn(&Value) -> bool { move |m| m["type"] == "ack" && m["id"] == id }

/// Smoke test through the spawned binary on the committed `flop_cancel` fixture (solve 43, cancel 44): §13.2's latency
/// bounds and the one-terminal rule end to end. The cancels are sent at moments the test observes from outside (a
/// progress report, a solve's ack), which do not pin the job's stage; stage placement is `cancel_between_iterations`'.
/// Every message is kept, in order, none dropped.
#[test]
fn flop_cancel_fixture_smoke() {
    let mut w = Worker::spawn(8);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let flop = fixture_lines("flop_cancel");
    // a cancel sent once the job reports an iteration: the ack within 50 ms, `cancelled` within one iteration plus one
    // exploitability pass (<= 1.0 s), never a second terminal, and a later cancel is `already_finished`
    w.send(&flop[0]);
    let got = collect(&w, 20 * S, |m| m["type"] == "progress" && m["stage"] == "solving" && m["iterations"].as_u64().unwrap() >= 1);
    assert_eq!(protocol_only(&got), ["ack 43 accepted"]);
    let t = Instant::now();
    w.send(&flop[1]);
    let got = collect(&w, S, ack_for("44"));
    assert!(t.elapsed() <= Duration::from_millis(50), "ack took {:?}", t.elapsed());
    assert_eq!(protocol_only(&got), ["ack 44 accepted"]);
    let got = collect(&w, S, result_for("43"));
    assert_eq!(protocol_only(&got), ["result 43 cancelled"]);
    assert_eq!(protocol_only(&window(&w, Duration::from_millis(300))), Vec::<String>::new(), "no second terminal");
    w.send(&cancel("45", "43"));
    assert_eq!(protocol_only(&collect(&w, 5 * S, ack_for("45"))), ["ack 45 already_finished"]);
    // a cancel sent right behind its solve: acked in order, then the one terminal `cancelled`
    w.send(&with_id(&flop[0], "46"));
    w.send(&cancel("47", "46"));
    assert_eq!(protocol_only(&collect(&w, 2 * S, result_for("46"))), ["ack 46 accepted", "ack 47 accepted", "result 46 cancelled"]);
    // a cancel sent once the job reports Extracting: exactly one terminal, either `cancelled` behind the cancel's
    // `accepted`, or the job's own result when it completed first (behind the `accepted` of a cancel that came too
    // late for its last checkpoint, or ahead of an `already_finished`)
    w.send(&edit(&with_id(&flop[0], "48"), |v| v["target_bp"] = json!(u16::MAX)));
    let got = collect(&w, 20 * S, |m| m["type"] == "progress" && m["stage"] == "extracting");
    assert_eq!(protocol_only(&got), ["ack 48 accepted"]);
    w.send(&cancel("49", "48"));
    let got = collect(&w, 5 * S, result_for("48"));
    let status = got.last().unwrap()["status"].as_str().unwrap().to_string();
    match protocol_only(&got).as_slice() {
        [a, _] if a == "ack 49 accepted" => assert!(["cancelled", "ok", "best_so_far"].contains(&status.as_str()), "{status}"),
        [_] => {
            assert!(["ok", "best_so_far"].contains(&status.as_str()), "{status}");
            assert_eq!(protocol_only(&collect(&w, 5 * S, ack_for("49"))), ["ack 49 already_finished"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(protocol_only(&window(&w, Duration::from_millis(500))), Vec::<String>::new(), "no second terminal");
}

/// The brief's test with one fixture correction (report, deviation D1): `lock_river`'s OOP range, the hero
/// facing IP's all-in at node 2, is QQ and 66 (§13.2 `ev_convention_non_root_payoffs`), not AA, so the
/// hero's response is read at a 66 combo. Unlocked, IP bets QQ always and 54o half the time (66 is then
/// indifferent at one bluff in three) and 66 calls a quarter of the time (54o is then indifferent: it
/// wins the pot against 66's folds, 6(1 - c) of 9 OOP combos, and loses its bet against QQ's calls and
/// 66's, 3 + 6c of 9, so c = 1/4). Locked to 54o one time in five, IP bluffs one bet in six: 66's call
/// is worth -50, so 66 folds. "Lock during Solving" is placed by the barrier in process (fix round 1, review
/// P2.T14-I2), not inferred from a solve's ack.
#[test]
fn lock_lifecycle() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let lock = fixture_lines("lock_river");
    let flop = fixture_lines("flop_cancel");
    let qq = proto::combo_index(proto::Card::parse("Qc").unwrap(), proto::Card::parse("Qd").unwrap()) as usize;
    let o54 = proto::combo_index(proto::Card::parse("5c").unwrap(), proto::Card::parse("4d").unwrap()) as usize;
    let sixes = proto::combo_index(proto::Card::parse("6c").unwrap(), proto::Card::parse("6d").unwrap()) as usize;
    // unlocked reference: 66 calls about 25%
    w.send(&edit(&lock[1], |v| { v["id"] = json!("60"); v["spot"] = json!("ffff"); }));
    let free = result_of(&w, "60", 5 * S);
    assert_eq!(free["solution"]["locks_applied"], 0);
    let free_call = free["solution"]["nodes"][2]["probs"][sixes][1].as_f64().unwrap();   // nodes: [] oop, [check] ip, [check, allin] oop
    assert!((free_call - 0.25).abs() < 0.1, "66 calls {free_call} unlocked");
    // lock then the matching solve: locked rows unchanged, locks_applied 1, hero's response differs (66 folds against the 1-in-6 bluff frequency)
    w.send(&lock[0]);
    assert_eq!(ack_of(&w, "47")["status"], "staged");
    w.send(&lock[1]);
    let r = result_of(&w, "51", 5 * S);
    assert_eq!(r["status"], "ok");
    assert_eq!(r["solution"]["locks_applied"], 1);
    let ip = &r["solution"]["nodes"][1];
    assert_eq!(ip["probs"][qq], json!([0.0, 1.0]));
    assert!((ip["probs"][o54][1].as_f64().unwrap() - 0.2).abs() < 1e-3);
    let locked_call = r["solution"]["nodes"][2]["probs"][sixes][1].as_f64().unwrap();
    assert!(locked_call < 0.05, "66 calls {locked_call} against the locked range");
    // a lock with another spot: the next solve with the fixture spot is lock_mismatch and the lock is discarded
    w.send(&edit(&lock[0], |v| { v["id"] = json!("61"); v["spot"] = json!("abcd"); }));
    assert_eq!(ack_of(&w, "61")["status"], "staged");
    w.send(&with_id(&lock[1], "62"));
    assert_eq!(result_of(&w, "62", 5 * S)["error"]["code"], "lock_mismatch");
    w.send(&with_id(&lock[1], "63"));
    assert_eq!(result_of(&w, "63", 5 * S)["solution"]["locks_applied"], 0);
    // lock during Solving, in process: a job that carries a staged set, held by the barrier at its second iteration
    // boundary with the worker `Solving`; a lock is rejected `solve_in_progress` and stages nothing; the job is cancelled
    // there, and the set it consumed is gone: the next solve of the lock's spot runs unlocked
    let mut h = Harness::new();
    assert_eq!(h.send(&with_id(&lock[0], "80")), ["ack 80 staged replaced=false"]);
    h.barrier.arm(Site::Loop(LoopSite::Boundary(2)));
    assert_eq!(h.send(&edit(&with_id(&flop[0], "81"), |v| v["spot"] = lock_spot(&lock[0])))[0], "ack 81 accepted");
    let held = h.barrier.wait_held();
    assert!(held.contains(&Site::At(Checkpoint::LockApplied(1))), "the job applied the set it took: {held:?}");
    assert_eq!((h.state(), h.staged()), (WorkerState::Solving, false));
    assert_eq!(h.send(&with_id(&lock[0], "82")).last().map(String::as_str), Some("ack 82 rejected solve_in_progress"));
    assert_eq!((h.state(), h.staged()), (WorkerState::Solving, false), "the rejected lock staged nothing");
    assert_eq!(h.send(&cancel("83", "81")).last().map(String::as_str), Some("ack 83 accepted"));
    h.barrier.release();
    assert_eq!(briefs(&h.until_result("81")), ["result 81 cancelled"]);
    h.send(&with_id(&lock[1], "84"));
    let r = h.until_result("84").pop().unwrap();
    assert_eq!((r["status"].as_str(), r["solution"]["locks_applied"].as_u64()), (Some("ok"), Some(0)), "the consumed set is gone");
    // through the binary: a lock consumed by a cancelled solve is gone
    w.send(&with_id(&lock[0], "67"));
    assert_eq!(ack_of(&w, "67")["status"], "staged");
    w.send(&edit(&with_id(&flop[0], "68"), |v| v["spot"] = lock_spot(&lock[0])));
    w.send(r#"{"type":"cancel","id":"69","target":"68"}"#);
    assert_eq!(result_of(&w, "68", 5 * S)["status"], "cancelled");
    w.send(&with_id(&lock[1], "70"));
    assert_eq!(result_of(&w, "70", 5 * S)["solution"]["locks_applied"], 0);
    w.send(r#"{"type":"shutdown","id":"71"}"#);
    assert_eq!(ack_of(&w, "71")["status"], "accepted");
    // liveness-bounded, not a short fixed timeout: a concurrent gate can starve this exit past 2 s without a defect
    assert_eq!(w.wait_exit(LIVENESS), Some(0));
}
fn lock_spot(line: &str) -> Value { serde_json::from_str::<Value>(line).unwrap()["spot"].clone() }
