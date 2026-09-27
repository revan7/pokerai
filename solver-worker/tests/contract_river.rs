//! §13.2 river contracts through the spawned worker binary (`common::Worker`), plus `ev_conservation`
//! in process through the adapter's own mapping (IP's root-range EV is not an actor-owned wire export).
//! Every tolerance below is the spec's (§13.2) unless its comment says otherwise; none is loosened to pass.
mod common;
use common::{fixture_lines, Worker};
use proto::{combo_index, Card, Range1326};
use serde_json::{json, Value};
use std::time::Duration;

const S: Duration = Duration::from_secs(1);
fn c(s: &str) -> Card { Card::parse(s).unwrap() }
fn ci(a: &str, b: &str) -> usize { combo_index(c(a), c(b)) as usize }
fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn ready(w: &Worker) { assert_eq!(w.recv(5 * S).unwrap()["type"], "ready"); }
/// 20 s is a liveness bound, not a timing assertion: every solve below reaches its target in well under a second.
fn result_of(w: &Worker, id: &str) -> Value { w.recv_until(20 * S, |m| m["type"] == "result" && m["id"] == id).unwrap_or_else(|| panic!("no result {id}")) }
fn node<'a>(r: &'a Value, path: &Value) -> &'a Value { r["solution"]["nodes"].as_array().unwrap().iter().find(|n| &n["path"] == path).unwrap_or_else(|| panic!("no node {path}")) }
/// Range-level EV of the actor at a node: sum_c w_c * sum_a p_ca * ev_ca / sum_c w_c over available combos.
/// Raw range weights stand in for compatible support because no combo of either range below shares a card
/// with the other range (AA against QQ and 54o), so every combo's compatible opponent mass is the same.
fn range_ev(n: &Value, weights: &[f64]) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    for (i, w) in weights.iter().enumerate() {
        if *w <= 0.0 || n["available"][i] != true { continue; }
        let p = n["probs"][i].as_array().unwrap(); let e = n["ev_chips"][i].as_array().unwrap();
        num += w * p.iter().zip(e).map(|(p, e)| p.as_f64().unwrap() * e.as_f64().unwrap()).sum::<f64>(); den += w;
    }
    assert!(den > 0.0, "no available combo in the range at this node");
    num / den
}
fn weights_of(v: &Value, key: &str) -> Vec<f64> { v[key].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect() }
fn vec1326(entries: &[(usize, f32)]) -> Vec<f32> { let mut v = vec![0.0f32; 1326]; for (i, w) in entries { v[*i] = *w; } v }

#[test]
fn river_polarized_vs_bluffcatcher_analytic() {
    let mut w = Worker::spawn(4); ready(&w);
    let line = &fixture_lines("river_two_combo")[0];
    w.send(line);
    let r = result_of(&w, "41");
    assert_eq!(r["status"], "ok");
    let req: Value = serde_json::from_str(line).unwrap();
    // <= 0.1% of the 100 pot (§13.2); the fixture's own target is 10 bp, so `ok` already implies it.
    assert!(r["solution"]["exploitability_chips"].as_f64().unwrap() <= 0.1);
    let ip = node(&r, &json!([{"kind": "check"}]));
    // QQ (the nuts) bets 100% (§13.2); 0.97 leaves 3 pp of the tolerance the mixed frequencies get.
    for qq in [ci("Qc", "Qd"), ci("Qc", "Qh"), ci("Qd", "Qh")] { assert!(ip["probs"][qq][1].as_f64().unwrap() > 0.97); }
    // A pot bet lays the caller 2:1, so value : bluff = 2 : 1: QQ's mass 3 carries a bluff mass 1.5 of 54o's 3 (50%).
    let bluff: f64 = (0..1326).filter(|i| req["ip_range"][*i] == 0.25).map(|i| ip["probs"][i][1].as_f64().unwrap()).sum::<f64>() / 12.0;
    assert!((bluff - 0.5).abs() <= 0.03, "54o bluff frequency {bluff}");   // 50 +- 3 pp (§13.2)
    let oop = node(&r, &json!([{"kind": "check"}, {"kind": "allin", "to": 100}]));
    // A 100 bluff for a 100 pot is indifferent when the caller calls pot / (pot + bet) = 50%.
    let call = oop["probs"][ci("Ac", "Ad")][1].as_f64().unwrap();
    assert!((call - 0.5).abs() <= 0.03, "AA call frequency {call}");       // 50 +- 3 pp (§13.2)
    // Range EV, +- 1 chip (§13.2). IP 75 at its node (OOP always checks, so IP's node carries the whole root range):
    // QQ wins 100 + 0.5 * 100 = 150, 54o is indifferent at 0, (3 * 150 + 3 * 0) / 6 = 75. OOP 100 - 75 = 25 at the root.
    assert!((range_ev(ip, &weights_of(&req, "ip_range")) - 75.0).abs() <= 1.0);
    assert!((range_ev(node(&r, &json!([])), &weights_of(&req, "oop_range")) - 25.0).abs() <= 1.0);
}

#[test]
fn ev_convention_non_root_payoffs() {
    let mut w = Worker::spawn(4); ready(&w);
    let lock = fixture_lines("lock_river");
    w.send(&lock[0]);
    // Spec deviation recorded by the brief (§13.2 names AA; on Qs Jd 7h 3c 2d three queens beat aces): OOP QQ and 66.
    let oop = vec1326(&[(ci("Qc", "Qd"), 1.0), (ci("Qc", "Qh"), 1.0), (ci("Qd", "Qh"), 1.0), (ci("6c", "6d"), 1.0), (ci("6c", "6h"), 1.0), (ci("6c", "6s"), 1.0), (ci("6d", "6h"), 1.0), (ci("6d", "6s"), 1.0), (ci("6h", "6s"), 1.0)]);
    w.send(&edit(&lock[1], |v| { v["id"] = json!("80"); v["oop_range"] = json!(oop); }));
    let r = result_of(&w, "80");
    assert_eq!((r["status"].as_str(), r["solution"]["locks_applied"].as_u64()), (Some("ok"), Some(1)));
    // IP's strategy is locked (QQ bets 100%, 54o 20%), so OOP's call values are exact functions of the lock, not of
    // convergence: 1e-3 chips (§13.2) is f32 roundoff headroom at magnitudes of 300 (one ULP there is 3e-5).
    let facing = node(&r, &json!([{"kind": "check"}, {"kind": "allin", "to": 100}]));
    for qq in [ci("Qc", "Qd"), ci("Qc", "Qh"), ci("Qd", "Qh")] {
        assert_eq!(facing["ev_chips"][qq][0], 0.0);                                                   // fold = 0
        assert!((facing["ev_chips"][qq][1].as_f64().unwrap() - 200.0).abs() <= 1e-3, "QQ call EV {}", facing["ev_chips"][qq][1]);   // equity 1 * 300 - 100
    }
    let e66 = facing["ev_chips"][ci("6c", "6d")][1].as_f64().unwrap();
    assert!((e66 + 50.0).abs() <= 1e-3, "66 call EV {e66}");                                         // 0.6 / 3.6 * 300 - 100
    // turn-root spot with a check line: fold rows are 0 at IP's facing node and every EV is bounded by the stakes
    let basic = fixture_lines("basic_turn_std_request")[0].clone();
    w.send(&edit(&basic, |v| v["id"] = json!("81")));
    let r = result_of(&w, "81");
    assert_eq!(r["status"], "ok", "{}", r["error"]);
    let nodes = r["solution"]["nodes"].as_array().unwrap();
    let ip_facing = nodes.iter().find(|n| n["actor"] == "ip" && n["actions"][0]["kind"] == "fold").unwrap();
    for i in 0..1326 { if ip_facing["available"][i] == true { assert_eq!(ip_facing["ev_chips"][i][0], 0.0); } }
    let req: Value = serde_json::from_str(&basic).unwrap();
    let (pot, eff) = (req["pot"].as_f64().unwrap(), req["stack_oop"].as_f64().unwrap().min(req["stack_ip"].as_f64().unwrap()));
    // f32 roundoff of the library's EV arithmetic, which reaches the lower bound exactly for a combo that loses every
    // showdown (the V1 fixture holds -735.0001 against -735): the pot-relative noise tolerance of the exploitability
    // ruling, 8 f32 epsilons of the chips in play (1.9e-3 chips at 2000).
    let tol = 8.0 * f64::from(f32::EPSILON) * (pot + 2.0 * eff);
    for n in nodes {
        let (lo, hi) = stake_bounds(n, pot, eff);
        for i in 0..1326 { for e in n["ev_chips"][i].as_array().unwrap() {
            let e = e.as_f64().unwrap();
            assert!(e >= lo - tol && e <= hi + tol, "EV {e} of combo {i} at {} outside the stakes [{lo}, {hi}]", n["path"]);
        } }
    }
}

/// The stakes of the actor's EV at a node of a street-root solve. §2: EV(a) = E[final stack | a] - the actor's
/// remaining stack R at the node, and every terminal leaves the actor a final stack between 0 and all the chips in
/// play, T = pot + 2 * eff, so every EV lies in [-R, T - R]. R = eff - the actor's contribution since the root, read
/// off the chip path (OOP opens the street and the actors alternate; a wager's `to` is the actor's total contribution,
/// a call matches the opponent's). At the street root this is [-900, 1100] for `basic_turn_std`; deeper it moves
/// with the actor's commitment (up to 1285.8 in the V1 fixture, where OOP has 413 committed: bound 1513).
fn stake_bounds(n: &Value, pot: f64, eff: f64) -> (f64, f64) {
    let (mut contrib, mut actor) = ([0.0f64; 2], 0);
    for a in n["path"].as_array().unwrap() {
        match a["kind"].as_str().unwrap() {
            "check" => {}
            "call" => contrib[actor] = contrib[actor ^ 1],
            "bet" | "raise" | "allin" => contrib[actor] = a["to"].as_f64().unwrap(),
            other => panic!("{other} does not continue the street at {}", n["path"]),
        }
        actor ^= 1;
    }
    assert_eq!(n["actor"], ["oop", "ip"][actor], "actor at {}", n["path"]);
    let r = eff - contrib[actor];
    (-r, pot + 2.0 * eff - r)
}

#[test]
fn ev_conservation() {
    // identical full ranges, river_std_v1 at 100/100, no rake: EV_OOP + EV_IP = pot +- 0.5% at the root (library API, adapter mapping)
    use postflop_solver::*;
    use solver_worker::{cards, tree_build};
    let case: Value = fixture_lines("materialization_cases").iter().map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|c| c["case"] == "river_std_v1_100_100").unwrap();
    let tree: proto::EffectiveTree = serde_json::from_value(case["tree"].clone()).unwrap();
    let board = ["Qs", "Jd", "7h", "3c", "2d"].map(c);
    let mut full = Range1326([1.0; 1326]);
    for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { full.0[i] = 0.0; } }
    let action_tree = tree_build::build(&tree, 100, 100, 0.0, 0, &[]).unwrap();
    let (flop, turn, river) = cards::board_to_lib(&board).unwrap();
    let mut game = PostFlopGame::with_config(CardConfig { range: [cards::range_to_lib(&full).unwrap(), cards::range_to_lib(&full).unwrap()], flop, turn, river }, action_tree).unwrap();
    game.allocate_memory(false);
    solve(&mut game, 1000, 0.1, false);
    game.cache_normalized_weights();
    let total: f32 = (0..2).map(|p| compute_average(&game.expected_values(p), game.normalized_weights(p))).sum();
    assert!((total - 100.0).abs() <= 0.5, "EV_OOP + EV_IP = {total}");                                // 0.5% of the 100 pot (§13.2)
}

#[test]
fn rake_cap_applied() {
    let mut w = Worker::spawn(4); ready(&w);
    let base = &fixture_lines("river_two_combo")[0];
    // check-only tree: both menus empty, the only line is check-check (matched pot 100)
    let check_only = |v: &mut Value| {
        v["tree"]["menus"]["river"]["ip"]["bet"] = json!([]);
        v["tree"]["materialized"] = json!([{"path": [], "street": "river", "actor": "oop", "actions": [{"kind": "check"}], "terminal_pots": [null]},
                                           {"path": [0], "street": "river", "actor": "ip", "actions": [{"kind": "check"}], "terminal_pots": [100]}]);
        v["oop_range"] = json!(vec1326(&[(ci("Ac", "Ad"), 1.0), (ci("6c", "6d"), 1.0)]));
    };
    w.send(&edit(base, |v| { check_only(v); v["id"] = json!("90"); }));
    let unraked = result_of(&w, "90");
    w.send(&edit(base, |v| { check_only(v); v["id"] = json!("91"); v["rake_rate"] = json!(0.05); v["rake_cap_mchips"] = json!(3000); }));
    let raked = result_of(&w, "91");
    w.send(&edit(base, |v| { check_only(v); v["id"] = json!("92"); v["rake_rate"] = json!(0.05); v["rake_cap_mchips"] = json!(0); }));   // cap 0 = unraked (library: both must be > 0)
    let cap_zero = result_of(&w, "92");
    for r in [&unraked, &raked, &cap_zero] { assert_eq!(r["status"], "ok", "{}", r["error"]); }
    for combo in [ci("Ac", "Ad"), ci("6c", "6d")] {
        let u = node(&unraked, &json!([]))["ev_chips"][combo][0].as_f64().unwrap();
        let r = node(&raked, &json!([]))["ev_chips"][combo][0].as_f64().unwrap();
        let z = node(&cap_zero, &json!([]))["ev_chips"][combo][0].as_f64().unwrap();
        assert!(u > 0.0);
        // 5% of 100 = 5 capped at 3: a won showdown pays 97 instead of 100, so every check EV scales by exactly 0.97.
        // Check-only lines involve no strategy, so the values are exact up to f32 roundoff (1e-3 of a ratio near 1).
        assert!((r / u - 0.97).abs() <= 1e-3, "raked/unraked = {}", r / u);
        assert!((z - u).abs() <= 1e-3);
    }
}

#[test]
fn combo_matrix_two_named() {
    let mut w = Worker::spawn(4); ready(&w);
    let base = &fixture_lines("river_two_combo")[0];
    // OOP AsKs only, IP 7h7d only on Qs Jd 7c 3c 2d: IP holds trips and bets, OOP holds nothing and folds
    w.send(&edit(base, |v| { v["id"] = json!("95"); v["board"] = json!(["Qs", "Jd", "7c", "3c", "2d"]);
        v["oop_range"] = json!(vec1326(&[(ci("As", "Ks"), 1.0)])); v["ip_range"] = json!(vec1326(&[(ci("7h", "7d"), 1.0)])); }));
    let r = result_of(&w, "95");
    assert_eq!(r["status"], "ok");
    let ip = node(&r, &json!([{"kind": "check"}]));
    let oop = node(&r, &json!([{"kind": "check"}, {"kind": "allin", "to": 100}]));
    assert_eq!((ip["probs"].as_array().unwrap().len(), ip["probs"][0].as_array().unwrap().len()), (1326, 2));
    for i in 0..1326 {
        let ip_row = ip["probs"][i].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect::<Vec<_>>();
        let oop_row = oop["probs"][i].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect::<Vec<_>>();
        if i == ci("7h", "7d") { assert!(ip["available"][i] == true && ip_row[1] > 0.99); }
        else { assert!(ip["available"][i] == false && ip_row == [0.0, 0.0]); }
        if i == ci("As", "Ks") { assert!(oop["available"][i] == true && oop_row[0] > 0.99); }
        else { assert!(oop["available"][i] == false && oop_row == [0.0, 0.0]); }
    }
    assert_ne!(ip["probs"][ci("7h", "7d")], oop["probs"][ci("As", "Ks")]);
}
