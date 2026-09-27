//! §7 watchdog, on the fake clock. `FakeClock` and `RecordingSink` are `engine::testing`, which exists only with the
//! `testing` feature (lib.rs) and which no workspace member enables, so without it this file compiles to nothing:
//! run it as `cargo test -p engine --features testing --test watchdog`.
#![cfg(feature = "testing")]

use engine::deadline::Deadlines;
use engine::testing::{FakeClock, RecordingSink};
use engine::watchdog::{Armed, SharedSink, Watchdog};
use proto::{Coverage, DecisionIdentity, EquitySummary, Phase, Recommendation, RecommendationEvent, Street, UnsupportedReason};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn identity() -> DecisionIdentity { DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 } }
fn fallback() -> Recommendation {
    Recommendation { identity: identity(), phase: Phase::Fast,
        coverage: Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: String::new() }, partial: vec![] },
        legal: vec![], actions: vec![], unresolved_mass: 0.0, range_mix: None,
        equity: EquitySummary { hero_combo_vs_each: vec![], hero_range_vs_each: vec![], per_pot_shares: vec![] },
        assumptions: engine::assumptions_stub(), experimental: None, exploit: None }
}
fn armed(sink: SharedSink, stage: &str) -> (Armed, Arc<AtomicBool>, Arc<AtomicBool>, Arc<AtomicBool>) {
    let d = Deadlines::for_request(0, Street::River, 10);
    let (delivered, terminal_seen, violation) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let a = Armed { identity: identity(), street_deadline_ms: d.street_deadline_ms, fire_ms: d.watchdog_fire_ms(),
        retained: Arc::new(Mutex::new(None)), fallback: fallback(), stage: Arc::new(Mutex::new(stage.to_string())),
        sink, delivered: delivered.clone(), terminal_seen: terminal_seen.clone(), street_violation: violation.clone() };
    (a, delivered, terminal_seen, violation)
}
/// Spins on the fake clock (the watchdog thread is woken by `FakeClock::set_ms`, not by wall time).
fn wait_for(events: &Arc<Mutex<Vec<engine::testing::Recorded>>>, n: usize) -> Vec<engine::testing::Recorded> {
    for _ in 0..100_000 {
        let g = events.lock().unwrap();
        if g.len() >= n { return g.clone(); }
        drop(g);
        std::thread::yield_now();
    }
    panic!("watchdog did not emit {n} event(s)");
}

#[test]
fn watchdog_emits_final_at_delivery_minus_100ms() {
    let clock = FakeClock::new();
    let (sink, events) = RecordingSink::new(clock.clone(), None);
    let sink: SharedSink = Arc::new(Mutex::new(Box::new(sink)));
    let wd = Watchdog::new(clock.clone());
    let (a, delivered, _terminal, violation) = armed(sink, "extracting");
    wd.arm(a);
    clock.set_ms(14_900);
    let ev = wait_for(&events, 1);
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
    assert!(violation.load(Ordering::SeqCst), "no terminal was seen by the street deadline");
}

#[test]
fn watchdog_disarm_retires_the_generation() {
    let clock = FakeClock::new();
    let (sink, events) = RecordingSink::new(clock.clone(), None);
    let sink: SharedSink = Arc::new(Mutex::new(Box::new(sink)));
    let wd = Watchdog::new(clock.clone());
    let (a, _delivered, terminal, violation) = armed(sink, "solving");
    terminal.store(true, Ordering::SeqCst);
    wd.arm(a);
    wd.disarm();
    clock.set_ms(20_000);
    for _ in 0..100_000 { std::thread::yield_now(); }
    assert!(events.lock().unwrap().is_empty(), "a retired generation emits nothing");
    assert!(!violation.load(Ordering::SeqCst), "a terminal was seen before the street deadline");
}

fn recording(clock: &Arc<FakeClock>) -> (SharedSink, Arc<Mutex<Vec<engine::testing::Recorded>>>) {
    let (sink, events) = RecordingSink::new(clock.clone(), None);
    (Arc::new(Mutex::new(Box::new(sink))), events)
}
/// Gives the watchdog thread the same chance to act that `watchdog_disarm_retires_the_generation` gives it.
fn settle() { for _ in 0..100_000 { std::thread::yield_now(); } }

#[test]
fn watchdog_delivers_the_retained_payload_as_the_final() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, delivered, terminal, violation) = armed(sink, "solving");
    let mut provisional = fallback();
    provisional.phase = Phase::Provisional;
    provisional.coverage = Coverage::Approximate { reasons: vec![] };
    *a.retained.lock().unwrap() = Some(provisional.clone());
    let retained = a.retained.clone();
    terminal.store(true, Ordering::SeqCst);
    wd.arm(a);
    clock.set_ms(14_900);
    let ev = wait_for(&events, 1);
    assert_eq!(ev[0].at_ms, 14_900);
    match &ev[0].event {
        RecommendationEvent::Final(r) => {
            assert_eq!((r.phase, &r.identity, &r.coverage), (Phase::Final, &identity(), &provisional.coverage), "the retained Provisional, delivered as the Final");
        }
        e => panic!("{e:?}"),
    }
    settle();
    assert_eq!(events.lock().unwrap().len(), 1, "exactly one Final");
    assert!(retained.lock().unwrap().is_none(), "the retained payload is consumed by its delivery");
    assert!(delivered.load(Ordering::SeqCst) && !violation.load(Ordering::SeqCst));
}

#[test]
fn watchdog_records_the_street_violation_at_the_street_deadline() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, _delivered, _terminal, violation) = armed(sink, "solving");
    wd.arm(a);
    clock.set_ms(1_999);
    settle();
    assert!(!violation.load(Ordering::SeqCst), "the street deadline (2 000 ms) has not passed");
    clock.set_ms(2_000);
    for _ in 0..100_000 { if violation.load(Ordering::SeqCst) { break; } std::thread::yield_now(); }
    assert!(violation.load(Ordering::SeqCst), "no first-attempt terminal by t0 + 2 s");
    assert!(events.lock().unwrap().is_empty(), "the violation is recorded, nothing is delivered before 14 900 ms");
    clock.set_ms(14_899);
    settle();
    assert!(events.lock().unwrap().is_empty(), "nothing is delivered before 14 900 ms");
    clock.set_ms(14_900);
    assert_eq!(wait_for(&events, 1)[0].at_ms, 14_900);
}

#[test]
fn watchdog_never_delivers_a_final_the_engine_already_delivered() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, delivered, terminal, _violation) = armed(sink, "extracting");
    wd.arm(a);
    terminal.store(true, Ordering::SeqCst);
    delivered.store(true, Ordering::SeqCst); // the engine's own Final claimed the delivery first
    clock.set_ms(20_000);
    settle();
    assert!(events.lock().unwrap().is_empty(), "one Final per request: the watchdog does not deliver a second");
}

#[test]
fn arming_retires_the_previous_generation() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (first, first_delivered, _, first_violation) = armed(sink.clone(), "building");
    let (second, second_delivered, _, _) = armed(sink, "solving");
    wd.arm(first);
    wd.arm(second);
    clock.set_ms(14_900);
    let ev = wait_for(&events, 1);
    settle();
    assert_eq!(events.lock().unwrap().len(), 1, "only the live generation delivers");
    match &ev[0].event {
        RecommendationEvent::Final(r) => match &r.coverage { Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } => assert_eq!(stage, "solving"), c => panic!("{c:?}") },
        e => panic!("{e:?}"),
    }
    assert!(second_delivered.load(Ordering::SeqCst));
    assert!(!first_delivered.load(Ordering::SeqCst) && !first_violation.load(Ordering::SeqCst), "a retired generation touches nothing");
}

#[test]
#[should_panic(expected = "after its Final was delivered")]
fn arming_a_request_after_its_fire_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, delivered, terminal, violation) = armed(sink.clone(), "solving");
    let (retained, stage) = (a.retained.clone(), a.stage.clone());
    wd.arm(a);
    clock.set_ms(14_900);
    wait_for(&events, 1);
    // the same request (its shared flags) armed again after the watchdog fired for it
    wd.arm(Armed { identity: identity(), street_deadline_ms: 22_000, fire_ms: 29_900, retained, fallback: fallback(), stage, sink,
        delivered, terminal_seen: terminal, street_violation: violation });
}

#[test]
#[should_panic(expected = "already fired")]
fn arming_an_identity_after_the_watchdog_fired_for_it_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, _, _, _) = armed(sink.clone(), "solving");
    wd.arm(a);
    clock.set_ms(14_900);
    wait_for(&events, 1);
    let (again, _, _, _) = armed(sink, "solving"); // fresh flags, but the decision identity that was already delivered
    wd.arm(again);
}

#[test]
#[should_panic(expected = "fallback")]
fn arming_with_another_decisions_fallback_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let (mut a, _, _, _) = armed(sink, "solving");
    a.fallback.identity.decision_id = 2;
    Watchdog::new(clock.clone()).arm(a);
}

#[test]
#[should_panic(expected = "DeadlineExceeded")]
fn arming_with_a_fallback_that_is_not_deadline_exceeded_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let (mut a, _, _, _) = armed(sink, "solving");
    a.fallback.coverage = Coverage::Exact;
    Watchdog::new(clock.clone()).arm(a);
}

#[test]
fn watchdog_waits_without_holding_the_sink() {
    let clock = FakeClock::new();
    let (sink, events) = recording(&clock);
    let wd = Watchdog::new(clock.clone());
    let (a, _delivered, _terminal, violation) = armed(sink.clone(), "solving");
    wd.arm(a);
    settle();
    assert!(sink.try_lock().is_ok(), "the sink is free while the watchdog waits for the street deadline");
    clock.set_ms(2_000);
    for _ in 0..100_000 { if violation.load(Ordering::SeqCst) { break; } std::thread::yield_now(); }
    assert!(violation.load(Ordering::SeqCst));
    settle();
    assert!(sink.try_lock().is_ok(), "the sink is free while the watchdog waits for the fire");
    clock.set_ms(14_900);
    assert_eq!(wait_for(&events, 1).len(), 1);
}

use std::sync::mpsc;

/// Blocks inside `emit` until the test releases it, so a fire can be caught in progress.
struct GatedSink { entered: mpsc::Sender<()>, release: mpsc::Receiver<()>, inner: RecordingSink }
impl engine::EventSink for GatedSink {
    fn emit(&mut self, ev: RecommendationEvent) {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        self.inner.emit(ev);
    }
}

#[test]
fn disarm_returns_only_after_a_fire_in_progress_has_emitted() {
    let clock = FakeClock::new();
    let (inner, events) = RecordingSink::new(clock.clone(), None);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let sink: SharedSink = Arc::new(Mutex::new(Box::new(GatedSink { entered: entered_tx, release: release_rx, inner })));
    let wd = Watchdog::new(clock.clone());
    let (a, _, _, _) = armed(sink, "solving");
    wd.arm(a);
    clock.set_ms(14_900);
    let mut inside = false;
    for _ in 0..100_000 { if entered_rx.try_recv().is_ok() { inside = true; break; } std::thread::yield_now(); }
    assert!(inside, "the watchdog did not start its emission");
    let returned = AtomicBool::new(false);
    let returned_early = std::thread::scope(|s| {
        s.spawn(|| { wd.disarm(); returned.store(true, Ordering::SeqCst); });
        settle();
        let early = returned.load(Ordering::SeqCst);
        release_tx.send(()).unwrap();
        early
    });
    assert!(!returned_early, "disarm returned while a fire of the generation it retires was still emitting");
    assert!(returned.load(Ordering::SeqCst));
    assert_eq!(events.lock().unwrap().len(), 1, "the fire that began before the disarm completes, once");
}

#[test]
#[should_panic(expected = "street deadline")]
fn arming_with_the_street_deadline_after_the_fire_is_a_bug() {
    let clock = FakeClock::new();
    let (sink, _events) = recording(&clock);
    let (mut a, _, _, _) = armed(sink, "solving");
    a.street_deadline_ms = a.fire_ms + 1;
    Watchdog::new(clock.clone()).arm(a);
}
