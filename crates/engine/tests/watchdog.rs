//! The watchdog of spec §7 on the fake clock.
//!
//! Synchronization (ruling 20-I2): every wait in this file is an acknowledgement, never a bounded yield loop, a spin or
//! a sleep. The fake clock acknowledges a thread's registration in `wait_until` (`wait_for_waiter`, `waiting`) and
//! holds its waiters at a boundary (`hold`/`release`); the recording sink acknowledges each emission
//! (`Recorder::wait_for`); the watchdog acknowledges the end of each generation thread, after its emission or its
//! retired return (`wait_for_ended_threads`). A negative assertion runs only after the work it is about is
//! acknowledged. Time comes only from the fake clock.

use engine::deadline::Deadlines;
use engine::testing::{FakeClock, Recorder, RecordingSink};
use engine::watchdog::{Armed, SharedSink, StreetDeadline, Watchdog};
use proto::{Coverage, DecisionIdentity, EquitySummary, Phase, Recommendation, RecommendationEvent, Street, UnsupportedReason};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

fn identity() -> DecisionIdentity { DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 } }
fn fallback() -> Recommendation {
    Recommendation { identity: identity(), phase: Phase::Fast,
        coverage: Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: String::new() }, partial: vec![] },
        legal: vec![], actions: vec![], unresolved_mass: 0.0, range_mix: None,
        equity: EquitySummary { hero_combo_vs_each: vec![], hero_range_vs_each: vec![], per_pot_shares: vec![] },
        assumptions: engine::assumptions_stub(), experimental: None, exploit: None }
}
/// A river request at t0 = 0: street deadline 2 000 ms, fire 14 900 ms.
fn armed(sink: SharedSink, stage: &str) -> (Armed, Arc<AtomicBool>, Arc<StreetDeadline>) {
    let d = Deadlines::for_request(0, Street::River, 10);
    let (delivered, street) = (Arc::new(AtomicBool::new(false)), Arc::new(StreetDeadline::new(d.street_deadline_ms)));
    let a = Armed { identity: identity(), street_deadline: street.clone(), fire_ms: d.watchdog_fire_ms(),
        retained: Arc::new(Mutex::new(None)), fallback: fallback(), stage: Arc::new(Mutex::new(stage.to_string())),
        sink, delivered: delivered.clone() };
    (a, delivered, street)
}
fn recording(clock: &Arc<FakeClock>) -> (SharedSink, Recorder) {
    let (sink, recorder) = RecordingSink::notifying(clock.clone(), None);
    (Arc::new(Mutex::new(Box::new(sink))), recorder)
}

#[test]
fn watchdog_emits_final_at_delivery_minus_100ms() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, delivered, street) = armed(sink, "extracting");
    wd.arm(a);
    clock.set_ms(14_900);
    wd.wait_for_ended_threads(1); // the fire is over: nothing more can be emitted
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

#[test]
fn watchdog_disarm_retires_the_generation() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, _delivered, street) = armed(sink, "solving");
    street.terminal_arrived(0);
    wd.arm(a);
    wd.disarm();
    clock.set_ms(20_000);
    wd.wait_for_ended_threads(1); // the retired thread woke at the street deadline and ended
    assert!(events.recorded().is_empty(), "a retired generation emits nothing");
    assert!(!street.violated(), "a terminal was seen before the street deadline");
}

#[test]
fn a_retired_generation_records_no_street_deadline() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, _delivered, street) = armed(sink, "solving");
    wd.arm(a);
    clock.wait_for_waiter(2_000);
    wd.disarm();
    clock.set_ms(20_000);
    wd.wait_for_ended_threads(1);
    assert!(events.recorded().is_empty(), "a retired generation emits nothing");
    assert!(!street.violated(), "a retired generation does not record reaching the street deadline");
}

#[test]
fn watchdog_delivers_the_retained_payload_as_the_final() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
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
    wd.wait_for_ended_threads(1);
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
    let wd = Watchdog::new(clock.clone());
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
    let wd = Watchdog::new(clock.clone());
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
    let wd = Watchdog::new(clock.clone());
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
    let wd = Watchdog::new(clock.clone());
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
        let wd = Watchdog::new(clock.clone());
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
    let wd = Watchdog::new(clock.clone());
    let (a, delivered, street) = armed(sink, "extracting");
    wd.arm(a);
    street.terminal_arrived(0);
    delivered.store(true, Ordering::SeqCst); // the engine's own Final claimed the delivery first
    clock.set_ms(20_000);
    wd.wait_for_ended_threads(1); // the fire found the delivery claimed and ended
    assert!(events.recorded().is_empty(), "one Final per request: the watchdog does not deliver a second");
}

#[test]
fn arming_retires_the_previous_generation() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (first, first_delivered, first_street) = armed(sink.clone(), "building");
    let (second, second_delivered, second_street) = armed(sink, "solving");
    wd.arm(first);
    wd.arm(second);
    clock.set_ms(14_900);
    wd.wait_for_ended_threads(2); // the retired first thread and the fired second one
    let ev = events.recorded();
    assert_eq!(ev.len(), 1, "only the live generation delivers");
    match &ev[0].event {
        RecommendationEvent::Final(r) => match &r.coverage { Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } => assert_eq!(stage, "solving"), c => panic!("{c:?}") },
        e => panic!("{e:?}"),
    }
    assert!(second_delivered.load(Ordering::SeqCst) && second_street.violated());
    assert!(!first_delivered.load(Ordering::SeqCst) && !first_street.violated(), "a retired generation touches nothing");
}

#[test]
#[should_panic(expected = "after its Final was delivered")]
fn arming_a_request_after_its_fire_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, delivered, street) = armed(sink.clone(), "solving");
    let (retained, stage) = (a.retained.clone(), a.stage.clone());
    wd.arm(a);
    clock.set_ms(14_900);
    events.wait_for(1);
    // the same request (its shared state) armed again after the watchdog fired for it
    wd.arm(Armed { identity: identity(), street_deadline: street, fire_ms: 29_900, retained, fallback: fallback(), stage, sink, delivered });
}

#[test]
#[should_panic(expected = "already fired")]
fn arming_an_identity_after_the_watchdog_fired_for_it_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
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
    let (mut a, _, _) = armed(sink, "solving");
    a.fallback.identity.decision_id = 2;
    Watchdog::new(clock.clone()).arm(a);
}

#[test]
#[should_panic(expected = "DeadlineExceeded")]
fn arming_with_a_fallback_that_is_not_deadline_exceeded_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let (mut a, _, _) = armed(sink, "solving");
    a.fallback.coverage = Coverage::Exact;
    Watchdog::new(clock.clone()).arm(a);
}

#[test]
fn watchdog_waits_without_holding_the_sink() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
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
    wd.wait_for_ended_threads(1);
    assert_eq!(events.recorded().len(), 1);
}

/// Blocks inside `emit` until the test releases it, so a fire can be caught in progress; notes when the emission ends.
struct GatedSink { entered: mpsc::Sender<()>, release: mpsc::Receiver<()>, inner: RecordingSink, order: Arc<Mutex<Vec<&'static str>>> }
impl engine::EventSink for GatedSink {
    fn emit(&mut self, ev: RecommendationEvent) {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
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
    let wd = Watchdog::new(clock.clone());
    let (a, _, _) = armed(sink, "solving");
    wd.arm(a);
    clock.set_ms(14_900);
    entered_rx.recv().unwrap(); // acknowledgement: the fire is inside `emit`, and stays there until released
    assert!(wd.retirement_would_block(), "a fire holds the generation lock through its emission, so a disarm now blocks");
    let (calling_tx, calling_rx) = mpsc::channel();
    std::thread::scope(|s| {
        let (wd, order) = (&wd, &order);
        s.spawn(move || {
            calling_tx.send(()).unwrap();
            wd.disarm();
            order.lock().unwrap().push("disarm returned");
        });
        calling_rx.recv().unwrap(); // acknowledgement: the disarm is being called while the fire is still emitting
        release_tx.send(()).unwrap();
    });
    assert_eq!(*order.lock().unwrap(), ["emitted", "disarm returned"], "disarm returned while a fire of the generation it retires was still emitting");
    wd.wait_for_ended_threads(1);
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
