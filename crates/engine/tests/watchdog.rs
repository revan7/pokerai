//! The watchdog of spec §7 on the fake clock.
//!
//! Synchronization (ruling 20-I2): every wait in this file is an acknowledgement, never a bounded yield loop, a spin or
//! a sleep. The fake clock acknowledges a thread's registration in `wait_until` (`wait_for_waiter`, `waiting`) and
//! holds its waiters at a boundary (`hold`/`release`); the recording sink acknowledges each emission
//! (`Recorder::wait_for`); the watchdog acknowledges the end of each generation thread, after its emission or its
//! retired return (`wait_for_ended_threads`). A negative assertion runs only after the work it is about is
//! acknowledged. Time comes only from the fake clock.
//!
//! Liveness (ruling 20-A). Every acknowledgement wait is bounded by `ACK_LIVENESS` (60 s) of wall time, after which
//! the test fails naming the acknowledgement it waited for, so a broken watchdog fails loudly instead of hanging the
//! gate. The bound is a test-liveness allowance, never a correctness condition: the watchdog's time is the fake
//! clock's, and every acknowledgement arrives within microseconds of the test driving it.

use engine::clock::Clock;
use engine::deadline::Deadlines;
use engine::identity::IdentityState;
use engine::testing::{FakeClock, Recorder, RecordingSink, ACK_LIVENESS};
use engine::watchdog::{Armed, Fired, SharedSink, StreetDeadline, Watchdog};
use proto::{Coverage, DecisionIdentity, EquitySummary, Phase, Recommendation, RecommendationEvent, Street, UnsupportedReason};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

fn identity() -> DecisionIdentity { DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 } }
fn fallback() -> Recommendation { fallback_of(identity()) }
fn fallback_of(identity: DecisionIdentity) -> Recommendation {
    Recommendation { identity, phase: Phase::Fast,
        coverage: Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: String::new() }, partial: vec![] },
        legal: vec![], actions: vec![], unresolved_mass: 0.0, range_mix: None,
        equity: EquitySummary { hero_combo_vs_each: vec![], hero_range_vs_each: vec![], per_pot_shares: vec![] },
        assumptions: engine::assumptions_stub(), experimental: None, exploit: None }
}
/// A session whose active decision is `identity()`: config 1, hand 1 at revision 1, decision 1.
fn session() -> Arc<Mutex<IdentityState>> {
    let mut s = IdentityState::new();
    s.set_config();
    s.begin_hand();
    assert_eq!(s.next_decision(), Some(identity()));
    Arc::new(Mutex::new(s))
}
/// A river request at t0 = 0: street deadline 2 000 ms, fire 14 900 ms, for the active decision of a session of its own.
fn armed(sink: SharedSink, stage: &str) -> (Armed, Arc<AtomicBool>, Arc<StreetDeadline>) {
    armed_at(sink, stage, identity(), session(), 0)
}
/// A river request for decision `id` of `session`, admitted at `t0_ms`: street deadline `t0 + 2 000` ms, fire
/// `t0 + 14 900` ms.
fn armed_at(sink: SharedSink, stage: &str, id: DecisionIdentity, session: Arc<Mutex<IdentityState>>, t0_ms: u64) -> (Armed, Arc<AtomicBool>, Arc<StreetDeadline>) {
    let d = Deadlines::for_request(t0_ms, Street::River, 10);
    let (delivered, street) = (Arc::new(AtomicBool::new(false)), Arc::new(StreetDeadline::new(d.street_deadline_ms)));
    let a = Armed { identity: id.clone(), street_deadline: street.clone(), fire_ms: d.watchdog_fire_ms(),
        retained: Arc::new(Mutex::new(None)), fallback: Arc::new(Mutex::new(fallback_of(id))), stage: Arc::new(Mutex::new(stage.to_string())),
        sink, delivered: delivered.clone(), fired: Arc::default(), identity_state: session };
    (a, delivered, street)
}
fn recording(clock: &Arc<FakeClock>) -> (SharedSink, Recorder) {
    let (sink, recorder) = RecordingSink::notifying(clock.clone(), None);
    (Arc::new(Mutex::new(Box::new(sink))), recorder)
}
/// Shared, so that a bounded wait on it can run on a helper thread (`ended`).
fn watchdog(clock: &Arc<FakeClock>) -> Arc<Watchdog> { Arc::new(Watchdog::new(clock.clone())) }

/// Runs a blocking acknowledgement wait on a helper thread and fails the test, naming `what`, when it is not
/// acknowledged within `bound` of wall time (ruling 20-A: a test-liveness allowance, never a correctness condition).
/// A wait that never returns leaves its helper thread blocked, not the test.
fn acknowledged_within<T: Send + 'static>(what: &str, bound: Duration, wait: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || { let _ = tx.send(wait()); });
    rx.recv_timeout(bound).unwrap_or_else(|e| panic!("{what}: no acknowledgement within the {bound:?} liveness bound ({e})"))
}
/// The watchdog's completion seam, bounded by `ACK_LIVENESS`.
fn ended(wd: &Arc<Watchdog>, n: u64) {
    let wd = wd.clone();
    acknowledged_within(&format!("Watchdog::wait_for_ended_threads({n})"), ACK_LIVENESS, move || wd.wait_for_ended_threads(n));
}
/// A channel acknowledgement, bounded by `ACK_LIVENESS`.
fn received<T>(rx: &mpsc::Receiver<T>, what: &str) -> T {
    rx.recv_timeout(ACK_LIVENESS).unwrap_or_else(|e| panic!("{what}: no acknowledgement within the {ACK_LIVENESS:?} liveness bound ({e})"))
}
/// Runs `Watchdog::disarm(generation)` through the acknowledgement bound (ruling 20-A): a watchdog that mishandles the
/// generation lock fails the test naming this acknowledgement, instead of hanging the gate on an unbounded call.
fn disarmed(wd: &Arc<Watchdog>, generation: u64) {
    let wd = wd.clone();
    acknowledged_within("Watchdog::disarm returned (the generation lock was free)", ACK_LIVENESS, move || wd.disarm(generation));
}
/// Runs `Watchdog::arm` through the acknowledgement bound (ruling 20-A), for the same reason as `disarmed`.
fn armed_on(wd: &Arc<Watchdog>, a: Armed) {
    let wd = wd.clone();
    acknowledged_within("Watchdog::arm returned (the generation lock was free)", ACK_LIVENESS, move || wd.arm(a));
}

#[test]
fn watchdog_emits_final_at_delivery_minus_100ms() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, delivered, street) = armed(sink, "extracting");
    wd.arm(a);
    clock.set_ms(14_900);
    ended(&wd, 1); // the fire is over: nothing more can be emitted
    let ev = events.recorded();
    assert_eq!(ev.len(), 1, "exactly one Final");
    assert_eq!(ev[0].at_ms, 14_900);
    match &ev[0].event {
        RecommendationEvent::Final(r) => {
            assert_eq!(r.phase, Phase::Final);
            match &r.coverage { Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } => assert_eq!(stage, "extracting"), c => panic!("{c:?}") }
        }
        e => panic!("{e:?}"),
    }
    assert!(delivered.load(Ordering::SeqCst));
    assert!(street.violated(), "no terminal was seen by the street deadline");
}

/// Ruling 28-I6: the fallback is read at the fire, so what the engine wrote into it after arming (here an inherited
/// reason and the ranges used) is what the watchdog's `Final` carries. Ruling 28-I2: the fire records that `Final`, and
/// the time it was emitted at, for the engine's decision log.
#[test]
fn the_fire_delivers_the_fallback_as_last_refreshed_and_records_it() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, _delivered, _street) = armed(sink, "solving");
    let (slot, fired) = (a.fallback.clone(), a.fired.clone());
    wd.arm(a);
    let reason = proto::ApproxReason::UnconditionedCurrentStreet;
    {
        let mut refreshed = slot.lock().unwrap();
        refreshed.coverage = Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: String::new() }, partial: vec![reason.clone()] };
        refreshed.assumptions.ranges_used = vec![(proto::Seat(2), "AA".into(), 6.0)];
    }
    clock.set_ms(14_900);
    ended(&wd, 1);
    let ev = events.recorded();
    assert_eq!(ev.len(), 1);
    match &ev[0].event {
        RecommendationEvent::Final(r) => {
            assert_eq!(r.coverage, Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: "solving".into() }, partial: vec![reason] });
            assert_eq!(r.assumptions.ranges_used, vec![(proto::Seat(2), "AA".to_string(), 6.0)]);
            assert_eq!(*fired.lock().unwrap(), Some(Fired { at_ms: 14_900, rec: r.clone() }), "the fire records the Final it emitted, and when");
        }
        e => panic!("{e:?}"),
    }
}

#[test]
fn watchdog_disarm_retires_the_generation() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, _delivered, street) = armed(sink, "solving");
    street.terminal_arrived(0);
    let generation = wd.arm(a);
    disarmed(&wd, generation);
    clock.set_ms(20_000);
    ended(&wd, 1); // the retired thread woke at the street deadline and ended
    assert!(events.recorded().is_empty(), "a retired generation emits nothing");
    assert!(!street.violated(), "a terminal was seen before the street deadline");
}

#[test]
fn a_retired_generation_records_no_street_deadline() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, _delivered, street) = armed(sink, "solving");
    let generation = wd.arm(a);
    clock.wait_for_waiter(2_000);
    disarmed(&wd, generation);
    clock.set_ms(20_000);
    ended(&wd, 1);
    assert!(events.recorded().is_empty(), "a retired generation emits nothing");
    assert!(!street.violated(), "a retired generation does not record reaching the street deadline");
}

#[test]
fn watchdog_delivers_the_retained_payload_as_the_final() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, delivered, street) = armed(sink, "solving");
    let mut provisional = fallback();
    provisional.phase = Phase::Provisional;
    provisional.coverage = Coverage::Approximate { reasons: vec![] };
    provisional.assumptions.reached_bp = Some(40);
    provisional.assumptions.cache = "hit".into();
    *a.retained.lock().unwrap() = Some(provisional.clone());
    let retained = a.retained.clone();
    street.terminal_arrived(0);
    wd.arm(a);
    clock.set_ms(14_900);
    ended(&wd, 1);
    let ev = events.recorded();
    assert_eq!(ev.len(), 1, "exactly one Final");
    assert_eq!(ev[0].at_ms, 14_900);
    let mut expected = provisional;
    expected.phase = Phase::Final;
    assert_eq!(ev[0].event, RecommendationEvent::Final(expected), "the retained Provisional, whole, delivered as the Final");
    assert!(retained.lock().unwrap().is_none(), "the retained payload is consumed by its delivery");
    assert!(delivered.load(Ordering::SeqCst) && !street.violated());
}

#[test]
fn watchdog_records_the_street_violation_at_the_street_deadline() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, _delivered, street) = armed(sink, "solving");
    wd.arm(a);
    clock.wait_for_waiter(2_000);
    clock.set_ms(1_999);
    assert_eq!(clock.waiting(), [2_000], "at 1 999 ms the watchdog is still in its street wait");
    assert!(!street.violated(), "the street deadline (2 000 ms) has not passed");
    clock.set_ms(2_000);
    clock.wait_for_waiter(14_900); // the watchdog has processed the street deadline and waits for the fire
    assert!(street.violated(), "no first-attempt terminal by t0 + 2 s");
    assert!(events.recorded().is_empty(), "the violation is recorded, nothing is delivered before 14 900 ms");
    clock.set_ms(14_899);
    assert_eq!(clock.waiting(), [14_900], "at 14 899 ms the watchdog is still waiting for the fire");
    assert!(events.recorded().is_empty(), "nothing is delivered before 14 900 ms");
    clock.set_ms(14_900);
    assert_eq!(events.wait_for(1)[0].at_ms, 14_900);
}

/// Ruling 20-I1: a first-attempt terminal that arrives 1 ms after the street deadline and is published while the
/// watchdog is still held in its street wait is a street violation.
#[test]
fn a_terminal_one_ms_late_is_a_violation_even_when_published_before_the_watchdog_resumes() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, _delivered, street) = armed(sink, "solving");
    wd.arm(a);
    clock.wait_for_waiter(2_000);
    clock.hold();
    clock.set_ms(2_001);
    street.terminal_arrived(2_001); // the terminal arrives at 2 001 ms, before the watchdog resumes
    assert!(street.violated(), "judged from the arrival time, before the watchdog has even resumed");
    clock.release();
    clock.wait_for_waiter(14_900); // the watchdog has processed the street deadline
    assert!(street.violated(), "the first-attempt terminal arrived 1 ms after the street deadline");
    assert_eq!(street.terminal_arrival_ms(), Some(2_001));
}

/// Ruling 20-I1, the on-time counterpart: a terminal that arrives at the street deadline, published while the watchdog
/// is still held in its street wait, records no violation.
#[test]
fn a_terminal_at_the_street_deadline_is_on_time_even_when_the_watchdog_resumes_after_it() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, _delivered, street) = armed(sink, "solving");
    wd.arm(a);
    clock.wait_for_waiter(2_000);
    clock.hold();
    clock.set_ms(2_000);
    street.terminal_arrived(2_000);
    clock.release();
    clock.wait_for_waiter(14_900);
    assert!(!street.violated(), "the first-attempt terminal arrived by the street deadline");
}

#[test]
fn an_on_time_terminal_records_no_street_violation() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, _delivered, street) = armed(sink, "solving");
    wd.arm(a);
    clock.wait_for_waiter(2_000);
    clock.set_ms(1_500);
    street.terminal_arrived(1_500);
    assert!(street.terminal_seen());
    clock.set_ms(2_000);
    clock.wait_for_waiter(14_900);
    assert!(!street.violated(), "the first-attempt terminal arrived at 1 500 ms");
}

/// Ruling 20-I1: compliance is judged from the arrival time, not from the order of the publication and the watchdog's
/// record. The watchdog processes the street deadline with no terminal published; then a terminal that had arrived on
/// time (1 999 ms) is published: no violation. The same with a late arrival (2 500 ms): a violation.
#[test]
fn compliance_is_judged_from_the_arrival_time_whenever_the_terminal_is_published() {
    for (arrival_ms, late) in [(1_999, false), (2_500, true)] {
        let clock = FakeClock::new();
        let (sink, _events) = recording(&clock);
        let wd = watchdog(&clock);
        let (a, _delivered, street) = armed(sink, "solving");
        wd.arm(a);
        clock.wait_for_waiter(2_000);
        clock.set_ms(2_600);
        clock.wait_for_waiter(14_900);
        assert!(street.violated() && !street.terminal_seen(), "the live generation reached the deadline with no terminal published");
        street.terminal_arrived(arrival_ms);
        assert_eq!(street.violated(), late, "a terminal that arrived at {arrival_ms} ms, published at 2 600 ms");
        street.terminal_arrived(1_000);
        assert_eq!(street.terminal_arrival_ms(), Some(arrival_ms), "the first publication stands");
    }
}

#[test]
fn watchdog_never_delivers_a_final_the_engine_already_delivered() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, delivered, street) = armed(sink, "extracting");
    let fired = a.fired.clone();
    wd.arm(a);
    street.terminal_arrived(0);
    delivered.store(true, Ordering::SeqCst); // the engine's own Final claimed the delivery first
    clock.set_ms(20_000);
    ended(&wd, 1); // the fire found the delivery claimed and ended
    assert!(events.recorded().is_empty(), "one Final per request: the watchdog does not deliver a second");
    assert!(fired.lock().unwrap().is_none(), "nothing delivered, nothing recorded");
}

#[test]
fn arming_retires_the_previous_generation() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (first, first_delivered, first_street) = armed(sink.clone(), "building");
    let (second, second_delivered, second_street) = armed(sink, "solving");
    wd.arm(first);
    armed_on(&wd, second);
    clock.set_ms(14_900);
    ended(&wd, 2); // the retired first thread and the fired second one
    let ev = events.recorded();
    assert_eq!(ev.len(), 1, "only the live generation delivers");
    match &ev[0].event {
        RecommendationEvent::Final(r) => match &r.coverage { Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } => assert_eq!(stage, "solving"), c => panic!("{c:?}") },
        e => panic!("{e:?}"),
    }
    assert!(second_delivered.load(Ordering::SeqCst) && second_street.violated());
    assert!(!first_delivered.load(Ordering::SeqCst) && !first_street.violated(), "a retired generation touches nothing");
}

/// Final review I1: the watchdog is armed at admission, so a request can be retired by its successor's `arm` while
/// `engine-main` still serves it. That request's own `disarm` then retires nothing: the successor stays live and
/// delivers its `Final` at its fire, and the retired generation emits nothing. A `disarm` of the live generation
/// retires it. A stopped watchdog refuses `try_arm` and starts nothing.
#[test]
fn disarming_a_retired_generation_leaves_the_newer_one_live() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (first, first_delivered, _) = armed(sink.clone(), "building");
    let (second, second_delivered, _) = armed(sink.clone(), "queued");
    let first_generation = wd.arm(first);
    let second_generation = wd.arm(second);
    assert!(second_generation > first_generation);
    disarmed(&wd, first_generation); // the served request retires its own generation, already retired by the newer arm
    clock.set_ms(14_900);
    ended(&wd, 2);
    let ev = events.recorded();
    assert_eq!(ev.len(), 1, "the newer generation is still live and fires");
    assert!(matches!(&ev[0].event, RecommendationEvent::Final(r) if r.coverage == Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: "queued".into() }, partial: vec![] }));
    assert!(second_delivered.load(Ordering::SeqCst) && !first_delivered.load(Ordering::SeqCst));
    // the live generation's own disarm retires it
    let later = session();
    let later_id = later.lock().unwrap().next_decision().unwrap(); // the watchdog already fired for `identity()`
    let (third, third_delivered, _) = armed_at(sink.clone(), "solving", later_id, later, 20_000);
    let third_generation = wd.arm(third);
    disarmed(&wd, third_generation);
    clock.set_ms(40_000);
    ended(&wd, 3);
    assert_eq!(events.recorded().len(), 1, "a disarmed live generation emits nothing");
    assert!(!third_delivered.load(Ordering::SeqCst));
    // a stopped watchdog refuses `try_arm` and starts nothing
    let stopper = wd.clone();
    acknowledged_within("Watchdog::stop", ACK_LIVENESS, move || stopper.stop());
    let (late, _, _) = armed(sink, "building");
    assert!(wd.try_arm(late).is_err(), "a stopped watchdog refuses to arm");
    assert_eq!(wd.ended_thread_count(), 3, "no thread was started for it");
}

#[test]
#[should_panic(expected = "after its Final was delivered")]
fn arming_a_request_after_its_fire_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, delivered, street) = armed(sink.clone(), "solving");
    let (retained, stage) = (a.retained.clone(), a.stage.clone());
    wd.arm(a);
    clock.set_ms(14_900);
    events.wait_for(1);
    // the same request (its shared state) armed again after the watchdog fired for it
    wd.arm(Armed { identity: identity(), street_deadline: street, fire_ms: 29_900, retained, fallback: Arc::new(Mutex::new(fallback())), stage, sink, delivered, fired: Arc::default(),
        identity_state: session() });
}

#[test]
#[should_panic(expected = "already fired")]
fn arming_an_identity_after_the_watchdog_fired_for_it_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, _, _) = armed(sink.clone(), "solving");
    wd.arm(a);
    clock.set_ms(14_900);
    events.wait_for(1);
    let (again, _, _) = armed(sink, "solving"); // fresh state, but the decision identity that was already delivered
    wd.arm(again);
}

#[test]
#[should_panic(expected = "fallback")]
fn arming_with_another_decisions_fallback_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let (a, _, _) = armed(sink, "solving");
    a.fallback.lock().unwrap().identity.decision_id = 2;
    Watchdog::new(clock.clone()).arm(a);
}

#[test]
#[should_panic(expected = "DeadlineExceeded")]
fn arming_with_a_fallback_that_is_not_deadline_exceeded_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let (a, _, _) = armed(sink, "solving");
    a.fallback.lock().unwrap().coverage = Coverage::Exact;
    Watchdog::new(clock.clone()).arm(a);
}

#[test]
fn watchdog_waits_without_holding_the_sink() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = watchdog(&clock);
    let (a, _delivered, street) = armed(sink.clone(), "solving");
    wd.arm(a);
    clock.wait_for_waiter(2_000);
    assert!(sink.try_lock().is_ok(), "the sink is free while the watchdog waits for the street deadline");
    assert!(!wd.retirement_would_block(), "and so is the generation lock");
    clock.set_ms(2_000);
    clock.wait_for_waiter(14_900);
    assert!(street.violated());
    assert!(sink.try_lock().is_ok(), "the sink is free while the watchdog waits for the fire");
    assert!(!wd.retirement_would_block(), "and so is the generation lock");
    clock.set_ms(14_900);
    ended(&wd, 1);
    assert_eq!(events.recorded().len(), 1);
}

/// Blocks inside `emit` until the test releases it, so a fire can be caught in progress; notes when the emission ends.
/// Its wait for the release is bounded like the test's own waits, so a test that fails before releasing it never
/// leaves the fire (and the generation lock it holds) stuck.
struct GatedSink { entered: mpsc::Sender<()>, release: mpsc::Receiver<()>, inner: RecordingSink, order: Arc<Mutex<Vec<&'static str>>> }
impl engine::EventSink for GatedSink {
    fn emit(&mut self, ev: RecommendationEvent) {
        self.entered.send(()).unwrap();
        received(&self.release, "GatedSink: the test released the emission");
        self.inner.emit(ev);
        self.order.lock().unwrap().push("emitted");
    }
}

#[test]
fn disarm_returns_only_after_a_fire_in_progress_has_emitted() {
    let clock = FakeClock::new();
    let (inner, events) = RecordingSink::notifying(clock.clone(), None);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let order = Arc::new(Mutex::new(Vec::new()));
    let sink: SharedSink = Arc::new(Mutex::new(Box::new(GatedSink { entered: entered_tx, release: release_rx, inner, order: order.clone() })));
    let wd = watchdog(&clock);
    let (a, _, _) = armed(sink, "solving");
    let generation = wd.arm(a);
    clock.set_ms(14_900);
    received(&entered_rx, "the fire entered emit"); // it stays there until released
    assert!(wd.retirement_would_block(), "a fire holds the generation lock through its emission, so a disarm now blocks");
    let (calling_tx, calling_rx) = mpsc::channel();
    let (returned_tx, returned_rx) = mpsc::channel();
    let (wd_thread, order_thread) = (wd.clone(), order.clone());
    std::thread::spawn(move || {
        calling_tx.send(()).unwrap();
        wd_thread.disarm(generation);
        order_thread.lock().unwrap().push("disarm returned");
        returned_tx.send(()).unwrap();
    });
    received(&calling_rx, "the disarm thread is calling disarm while the fire is still emitting");
    release_tx.send(()).unwrap();
    received(&returned_rx, "disarm returned after the fire's emission");
    assert_eq!(*order.lock().unwrap(), ["emitted", "disarm returned"], "disarm returned while a fire of the generation it retires was still emitting");
    ended(&wd, 1);
    assert_eq!(events.recorded().len(), 1, "the fire that began before the disarm completes, once");
}

#[test]
#[should_panic(expected = "street deadline")]
fn arming_with_the_street_deadline_after_the_fire_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let (mut a, _, _) = armed(sink, "solving");
    a.street_deadline = Arc::new(StreetDeadline::new(a.fire_ms + 1));
    Watchdog::new(clock.clone()).arm(a);
}

// --- Follow-up P2.W3: the fire checks that its decision is still the active one, under the identity lock and in the
// step that claims the request's `Final`, and hands the `Final` to the sink only after that lock is released. ---

/// Follow-up P2.W3 (spec 12: a stale event is discarded by engine-main; spec 13.3, the identity race): a decision
/// superseded just before its fire with no newer arm, by an undo or by a newer decision whose request has not armed yet
/// (it waits behind its predecessor's 1.5 s cancel window), gets nothing from its watchdog: no `Final`, its claim not
/// taken, nothing recorded as delivered, and the fire's thread ends. The newer decision, armed afterwards, is watched as
/// ever: its own fire delivers its `Final`.
#[test]
fn a_decision_superseded_just_before_its_fire_gets_no_final_and_the_newer_one_is_watched() {
    type Supersede = fn(&mut IdentityState);
    let cases: [(&str, Supersede); 2] = [
        ("an undo", |s| { s.mutate(); }),
        ("a newer decision not yet armed", |s| { s.next_decision().expect("a hand is in progress"); }),
    ];
    for (case, supersede) in cases {
        let clock = FakeClock::new();
        let (sink, events) = recording(&clock);
        let wd = watchdog(&clock);
        let (a, delivered, _street) = armed(sink.clone(), "solving");
        let (session, fired) = (a.identity_state.clone(), a.fired.clone());
        wd.arm(a);
        clock.wait_for_waiter(2_000);
        clock.set_ms(14_800);
        clock.wait_for_waiter(14_900); // the watchdog waits for its fire
        supersede(&mut session.lock().unwrap()); // 100 ms before the fire; nothing newer is armed
        assert!(!session.lock().unwrap().is_active(&identity()), "{case}: the armed decision is superseded");
        clock.set_ms(14_900);
        ended(&wd, 1); // the fire is over
        assert!(events.recorded().is_empty(), "{case}: a superseded decision gets no Final");
        assert!(!delivered.load(Ordering::SeqCst), "{case}: its Final claim is not taken");
        assert!(fired.lock().unwrap().is_none(), "{case}: nothing is recorded as delivered");
        // the newer decision (issued after the undo) is requested at 14 900 ms and arms the same watchdog: fire 29 800 ms
        let newer = { let mut s = session.lock().unwrap(); match s.active().cloned() { Some(active) => active, None => s.next_decision().expect("a hand is in progress") } };
        let (next, next_delivered, _) = armed_at(sink, "building", newer.clone(), session, 14_900);
        armed_on(&wd, next);
        clock.set_ms(29_800);
        ended(&wd, 2);
        let ev = events.recorded();
        assert_eq!(ev.len(), 1, "{case}: the newer decision's Final only");
        assert_eq!(ev[0].at_ms, 29_800, "{case}");
        assert!(matches!(&ev[0].event, RecommendationEvent::Final(r) if r.identity == newer
            && r.coverage == Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: "building".into() }, partial: vec![] }), "{case}: {:?}", ev[0].event);
        assert!(next_delivered.load(Ordering::SeqCst), "{case}");
    }
}

/// A sink that re-enters the engine from its callback, as a UI command handler on the delivery thread could: it notes
/// whether the identity lock and the stage slot (both `EngineCore` locks) are free, reads the active decision and
/// supersedes it. `try_lock`, so a callback run under either lock is recorded instead of deadlocking the test.
struct ReentrantSink { session: Arc<Mutex<IdentityState>>, stage: Arc<Mutex<String>>, seen: Arc<Mutex<Vec<String>>>, inner: RecordingSink }
impl engine::EventSink for ReentrantSink {
    fn emit(&mut self, ev: RecommendationEvent) {
        let stage_free = self.stage.try_lock().is_ok();
        let seen = match self.session.try_lock() {
            Ok(mut ids) => {
                let active = ids.active().is_some_and(|a| *a == identity());
                ids.mutate();
                format!("identity lock free, {} active, stage slot free {stage_free}", if active { "decision" } else { "no decision" })
            }
            Err(std::sync::TryLockError::WouldBlock) => format!("the identity lock was held during the callback, stage slot free {stage_free}"),
            Err(std::sync::TryLockError::Poisoned(p)) => panic!("identity lock poisoned: {p}"),
        };
        self.seen.lock().unwrap().push(seen);
        self.inner.emit(ev);
    }
}

/// Ruling 28-I1 at the fire (follow-up P2.W3): the identity check and the claim are made under the identity lock, and
/// the `Final` is handed to the sink only after that lock is released (the stage slot too), so a sink may re-enter the
/// engine from the watchdog's callback, read the active decision and supersede it: the fire completes, delivered, and
/// nothing deadlocks.
#[test]
fn a_sink_that_reads_the_identity_and_supersedes_from_the_fires_callback_completes() {
    let clock = FakeClock::new();
    let (inner, events) = RecordingSink::notifying(clock.clone(), None);
    let wd = watchdog(&clock);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (session, stage) = (session(), Arc::new(Mutex::new("solving".to_string())));
    let sink: SharedSink = Arc::new(Mutex::new(Box::new(ReentrantSink { session: session.clone(), stage: stage.clone(), seen: seen.clone(), inner })));
    let (mut a, delivered, _street) = armed(sink, "solving");
    (a.identity_state, a.stage) = (session.clone(), stage);
    let fired = a.fired.clone();
    wd.arm(a);
    clock.set_ms(14_900);
    ended(&wd, 1);
    assert_eq!(*seen.lock().unwrap(), ["identity lock free, decision active, stage slot free true"]);
    assert!(matches!(&events.recorded()[..], [e] if matches!(&e.event, RecommendationEvent::Final(r) if r.identity == identity())));
    assert!(delivered.load(Ordering::SeqCst) && fired.lock().unwrap().is_some(), "the Final was claimed and recorded before the callback");
    assert!(!session.lock().unwrap().is_active(&identity()), "the sink's supersession completed");
}

/// Ruling 20-A: an acknowledgement that never comes fails the test, naming it, instead of hanging the gate (the zero
/// bound stands for `ACK_LIVENESS` expiring; the helper's wait ends when the test's sender is dropped).
#[test]
#[should_panic(expected = "an acknowledgement that never comes: no acknowledgement within the 0ns liveness bound")]
fn an_acknowledgement_that_never_comes_fails_the_test_instead_of_hanging() {
    let (_never, rx) = mpsc::channel::<()>();
    acknowledged_within("an acknowledgement that never comes", Duration::ZERO, move || { let _ = rx.recv(); });
}

/// Ruling 29-I2: the watchdog owns its generation threads. `stop` wakes every one of them, the retired and the live,
/// wherever it waits on the clock, and returns only once each has ended: none emits, records a fired `Final` or reaches
/// its street deadline, and the clock does not move. A second `stop` does nothing, and arming after a stop is a bug.
#[test]
fn stop_wakes_and_joins_every_generation_thread() {
    let clock = FakeClock::new();
    let wd = watchdog(&clock);
    let (sink, recorder) = recording(&clock);
    let (retired, retired_delivered, retired_street) = armed(sink.clone(), "building");
    wd.arm(retired);
    let (live, live_delivered, live_street) = armed_at(sink.clone(), "solving", identity(), session(), 5);
    wd.arm(live);
    clock.wait_for_waiter(2_000);
    clock.wait_for_waiter(2_005);
    assert_eq!(wd.ended_thread_count(), 0, "both generation threads wait on the clock");
    let stopper = wd.clone();
    acknowledged_within("Watchdog::stop", ACK_LIVENESS, move || stopper.stop());
    assert_eq!(wd.ended_thread_count(), 2, "stop returned only after both generation threads ended");
    assert!(clock.waiting().is_empty() && clock.now_ms() == 0, "woken, not timed out: the clock never moved");
    assert!(recorder.recorded().is_empty() && !retired_delivered.load(Ordering::SeqCst) && !live_delivered.load(Ordering::SeqCst));
    assert!(!retired_street.violated() && !live_street.violated(), "neither reached its street deadline");
    let stopper = wd.clone();
    acknowledged_within("a second Watchdog::stop", ACK_LIVENESS, move || stopper.stop());
    assert_eq!(wd.ended_thread_count(), 2);
    let (late, _, _) = armed(sink, "building");
    let armed_after_stop = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wd.arm(late)));
    assert!(armed_after_stop.is_err(), "arming a stopped watchdog is a bug");
    assert_eq!(wd.ended_thread_count(), 2, "no thread was started for it");
}
