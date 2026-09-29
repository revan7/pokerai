//! Plan 4 Task 11 (spec 6, 7, 13.3): the `experimental` synthetic-root surrogate of a multiway decision.
//!
//! Spec 6's multiway row stays `Unsupported{MultiwayEv}` with no numeric EV in `actions`; beside it, and never inside
//! it, the `experimental` block holds an isolated heads-up solve at a synthetic root: the opponent is the seat whose
//! street-root public range has the highest range-vs-range equity against hero's, the pot is the current total pot,
//! both stacks are the smaller of hero's and that opponent's, the history is empty, the ranges are the two seats'
//! street-root public ranges (never hero-conditioned), hero is OOP iff hero precedes the opponent in postflop order,
//! and hero's advice is read, for hero's actual combo, at the synthetic root when OOP and at the node after OOP's check
//! when IP. The surrogate never reaches the cache or the snapshot store, and a surrogate that cannot run (an all-in
//! opponent, a failed or hung solve) leaves the multiway `Final` as it was, with a note naming why the block is absent.
//!
//! `experimental_surrogate_golden` is spec 13.3's golden of that name: its `golden/experimental_surrogate.json` was
//! recorded once with `POKERAI_RECORD_GOLDENS=1`, inspected, and is compared from then on.

// `pub`, so the shared helpers this binary never calls are reachable rather than dead code, with no lint filter.
pub mod support;

use engine::assemble::{empty_assumptions, unsupported, AssemblyCtx};
use engine::deadline::Deadlines;
use engine::identity::IdentityState;
use engine::ranges::{ExplicitRanges, RangeSource};
use engine::clock::Clock;
use engine::serve::{serve_request_with, LiveRequest, ServeSeams};
use engine::experimental::{select_opponent, NoOpponent, PairEquity};
use engine::testing::{FakeClock, RecordingSink};
use engine::watchdog::SharedSink;
use proto::{
    combo_index, Action, ActionAdvice, Card, Coverage, EquityMethod, HandState, Range1326, Recommendation, RecommendationEvent, Seat, Street, UnsupportedReason,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use support::{FlopRig, SeatRanges};

/// The note of a `Final` whose `experimental` block is absent (`experimental::ABSENT_NOTE` prefix).
fn absent_notes(rec: &Recommendation) -> Vec<&String> {
    rec.assumptions.notes.iter().filter(|n| n.starts_with(engine::experimental::ABSENT_NOTE)).collect()
}

/// The multiway `Final` exactly as the engine answered it before this task: `assemble::unsupported` with
/// `MultiwayEv{pot_eligible}`, the legal menu with no EV, the request's target and nothing else.
fn multiway_final_before_task_11(state: &HandState, id: &proto::DecisionIdentity, pot_eligible: u8) -> Recommendation {
    let d = core_model::derive(state);
    let hero_combo = state.hero_cards.map(|h| combo_index(h[0], h[1]));
    let ctx = AssemblyCtx { identity: id.clone(), legal: d.legal.clone(), hero_combo, bb_chips: state.config.bb_chips, equity: engine::equity::pending_summary(&[]) };
    let mut assumptions = empty_assumptions("");
    assumptions.target_bp = support::game_config().solver.target_bp;
    unsupported(&ctx, UnsupportedReason::MultiwayEv { pot_eligible }, vec![], assumptions)
}

fn final_of(events: &[RecommendationEvent]) -> &Recommendation {
    events.iter().rev().find_map(|e| match e { RecommendationEvent::Final(r) => Some(r), _ => None }).expect("a Final")
}

#[test]
fn experimental_surrogate_golden() {
    // three-way flop: hero BB (Seat 2), UTG (Seat 3) and BTN (Seat 0) still in.
    // Pot 300 at the decision, remaining stacks hero 700, UTG 900, BTN 400.
    let state = support::three_way_flop();
    let d = core_model::derive(&state);
    assert_eq!((d.pot, d.stacks_remaining[2], d.stacks_remaining[3], d.stacks_remaining[0]), (300, 700, 900, 400));
    let roots = support::street_root_public_ranges(&state); // unconditioned street-root ranges
    let cancel = AtomicBool::new(false);
    let opponent = engine::experimental::choose_opponent(state.hero, &roots.hero, &roots.others, &state.board, Duration::from_millis(500), &cancel).unwrap();
    let input = engine::experimental::surrogate_input(&d, &state, state.hero, opponent, Street::Flop, "flop_fast_v1").unwrap();
    assert_eq!(input.pot, d.pot, "the synthetic pot is the current total pot");
    assert_eq!(input.stack, d.stacks_remaining[state.hero.0 as usize].min(d.stacks_remaining[opponent.0 as usize]), "the synthetic stack is the minimum");
    assert_eq!(input.template_id, "flop_fast_v1", "same template as the street");
    assert!(matches!(input.hero_role, "oop" | "ip"));
    let run = support::run_three_way_flop_script(&state);
    let final_result = final_of(&run.events);
    // §6: MultiwayEv, no numeric EV in `actions`, the surrogate lives in its own block
    assert!(matches!(final_result.coverage, Coverage::Unsupported { reason: UnsupportedReason::MultiwayEv { .. }, .. }));
    assert!(final_result.actions.iter().all(|a| a.ev_bb.is_none()));
    let experimental = final_result.experimental.as_ref().expect("experimental block");
    assert_eq!(experimental.note, proto::EXPERIMENTAL_NOTE);
    assert_eq!(experimental.opponent, opponent);
    assert_eq!((experimental.pot, experimental.stack), (input.pot, input.stack));
    assert!(!experimental.actions.is_empty());
    // The block is separate: the main result is the multiway Final as it was, field for field.
    let mut main = final_result.clone();
    main.experimental = None;
    assert_eq!(main, multiway_final_before_task_11(&state, &run.id, 3));
    // The ranges are the street-root public ranges of hero and the opponent (OOP then IP), and the synthetic solve had
    // an empty history at the total pot and the minimum stack.
    let opp_public = roots.others.iter().find(|(s, _)| *s == opponent).unwrap().1.clone();
    let (oop, ip) = if input.hero_role == "oop" { ((state.hero, &roots.hero), (opponent, &opp_public)) } else { ((opponent, &opp_public), (state.hero, &roots.hero)) };
    let used = |(seat, r): (Seat, &Range1326)| (seat, core_ranges::range_to_string(r), core_ranges::mass(r));
    assert_eq!(experimental.ranges_used, [used(oop), used(ip)]);
    assert_eq!(run.solves.len(), 1);
    let sent = &run.solves[0];
    assert!(sent.history.is_empty(), "empty history");
    assert_eq!((sent.pot, sent.stack_oop, sent.stack_ip), (input.pot, input.stack, input.stack));
    // never Exact, never a snapshot, never a cache entry
    assert!(!matches!(final_result.coverage, Coverage::Exact));
    assert_eq!((run.snapshots, run.misses), (0, 0));
    assert_eq!(run.cache_entries, 0);
    let frozen = serde_json::json!({"opponent": experimental.opponent, "hero_role": experimental.hero_role, "pot": experimental.pot, "stack": experimental.stack,
        "template_id": experimental.template_id, "ranges_used": experimental.ranges_used, "note": experimental.note});
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/experimental_surrogate.json");
    if std::env::var_os("POKERAI_RECORD_GOLDENS").is_some() {
        std::fs::write(&path, serde_json::to_vec_pretty(&frozen).unwrap()).unwrap();
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("the committed golden {} is missing ({e}); record it once with POKERAI_RECORD_GOLDENS=1 and inspect it", path.display()));
    let expected: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(frozen, expected);
}

#[test]
fn surrogate_is_skipped_when_the_opponent_is_all_in_or_stackless() {
    let state = support::three_way_flop_with_all_in_opponent();
    let d = core_model::derive(&state);
    assert!(d.all_in[3] && d.stacks_remaining[3] == 0, "UTG is all-in");
    assert!(engine::experimental::surrogate_input(&d, &state, state.hero, Seat(3), Street::Flop, "flop_fast_v1").is_none());
    // the button is not all-in: its synthetic root is formed
    let btn = engine::experimental::surrogate_input(&d, &state, state.hero, Seat(0), Street::Flop, "flop_fast_v1").expect("the button has chips behind");
    assert_eq!((btn.pot, btn.stack, btn.hero_role), (260, 420, "oop"));
}

/// §6 synthetic root: total pot (this street's chips included), the smaller remaining stack, hero OOP iff hero precedes
/// the opponent in postflop order; `None` at another street than the decision's, against hero itself or a folded seat.
#[test]
fn the_synthetic_root_takes_the_total_pot_the_minimum_stack_and_the_postflop_order() {
    let state = support::three_way_flop();
    let d = core_model::derive(&state);
    let at = |opponent: u8| engine::experimental::surrogate_input(&d, &state, state.hero, Seat(opponent), Street::Flop, "flop_fast_v1");
    let btn = at(0).unwrap();
    assert_eq!((btn.hero, btn.opponent, btn.pot, btn.stack, btn.hero_role, btn.template_id.as_str(), btn.pot_eligible), (Seat(2), Seat(0), 300, 400, "oop", "flop_fast_v1", 3));
    let utg = at(3).unwrap();
    assert_eq!((utg.pot, utg.stack, utg.hero_role), (300, 700, "oop"));
    assert!(at(2).is_none(), "hero is not its own opponent");
    assert!(at(1).is_none(), "the small blind folded");
    assert!(engine::experimental::surrogate_input(&d, &state, state.hero, Seat(0), Street::Turn, "turn_std_v1").is_none(), "not the decision's street");
    // hero on the button acts last: in position against either opponent
    let ip = support::three_way_flop_ip_hero_bb_100();
    let d = core_model::derive(&ip);
    for opponent in [Seat(2), Seat(5)] {
        let input = engine::experimental::surrogate_input(&d, &ip, ip.hero, opponent, Street::Flop, "flop_fast_v1").unwrap();
        assert_eq!((input.pot, input.stack, input.hero_role), (950, 9_700, "ip"));
    }
}

/// §6 opponent choice: the seat whose public range has the highest range-vs-range equity against hero's public range;
/// hero is never its own opponent, and a seat whose equity is not computed (here: cancelled) is skipped, never guessed.
#[test]
fn the_opponent_is_the_seat_with_the_highest_range_vs_range_equity() {
    let state = support::three_way_flop();
    let roots = support::street_root_public_ranges(&state);
    assert_eq!(roots.others.iter().map(|(s, _)| *s).collect::<Vec<_>>(), [Seat(3), Seat(0)], "postflop order: UTG, then the button");
    let live = AtomicBool::new(false);
    let choose = |others: &[(Seat, Range1326)], cancel: &AtomicBool| engine::experimental::choose_opponent(state.hero, &roots.hero, others, &state.board, Duration::from_millis(500), cancel);
    assert_eq!(choose(&roots.others, &live), Some(Seat(0)), "AA over JJ against QQ");
    let reversed: Vec<(Seat, Range1326)> = roots.others.iter().rev().cloned().collect();
    assert_eq!(choose(&reversed, &live), Some(Seat(0)), "the order the seats come in does not decide");
    let with_hero: Vec<(Seat, Range1326)> = [(state.hero, roots.hero.clone())].into_iter().chain(roots.others.iter().cloned()).collect();
    assert_eq!(choose(&with_hero, &live), Some(Seat(0)), "hero is skipped");
    assert_eq!(choose(&roots.others[..1], &live), Some(Seat(3)));
    assert_eq!(choose(&[], &live), None);
    assert_eq!(choose(&roots.others, &AtomicBool::new(true)), None, "no seat's equity was computed: no opponent");
}

#[test]
fn surrogate_never_enters_cache_or_snapshots_on_any_street() {
    for state in [support::three_way_flop(), support::three_way_turn(), support::three_way_river()] {
        let street = core_model::derive(&state).street;
        let run = support::run_three_way_flop_script(&state);
        let last = final_of(&run.events);
        let block = last.experimental.as_ref().unwrap_or_else(|| panic!("{street:?}: {:?}", last.assumptions.notes));
        assert_eq!(block.template_id, support::three_way_template(&state), "{street:?}: the street's own template");
        assert!(matches!(last.coverage, Coverage::Unsupported { reason: UnsupportedReason::MultiwayEv { pot_eligible: 3 }, .. }), "{street:?}");
        assert!(last.actions.iter().all(|a| a.ev_bb.is_none() && a.frequency.is_none()), "{street:?}: no numeric EV in the main actions");
        assert_eq!((run.snapshots, run.misses), (0, 0), "{street:?}: no snapshot and no miss");
        assert_eq!(run.cache_entries, 0, "{street:?}: nothing on disk");
        // The main result's own assumptions are untouched: no cache route, template or tree of the surrogate's.
        assert_eq!((last.assumptions.cache.as_str(), last.assumptions.template_id.as_str(), last.assumptions.tree_signature.as_str()), ("miss", "", ""), "{street:?}");
        // The decision log records the multiway row (no street verdict, no template), as before.
        let record = run.records.last().expect("the Final is logged");
        assert_eq!((record.street, record.template_id.as_str(), record.street_violation, record.final_violation), (street, "", false, false));
    }
}

/// The surrogate uses the hand's real BB and the real hero combo, not 2 and not a blank reach.
#[test]
fn surrogate_uses_real_bb_and_the_actual_hero_combo_when_ip() {
    let state = support::three_way_flop_ip_hero_bb_100(); // bb_chips = 100, hero acts IP
    assert_eq!(state.config.bb_chips, 100);
    let case = support::surrogate_case(&state, &support::three_way_ranges());
    assert_eq!((case.input.opponent, case.input.hero_role), (Seat(5), "ip"), "the cutoff's KK has the highest equity against AA");
    let run = support::run_three_way_flop_script(&state);
    let last = final_of(&run.events);
    let experimental = last.experimental.as_ref().unwrap();
    assert_eq!(experimental.hero_role, "ip");
    // ev_bb is ev_chips / 100, never ev_chips / 2
    let ev_bb = experimental.actions.iter().find_map(|a| a.ev_bb).unwrap();
    assert!(ev_bb.abs() < experimental.stack as f32 / 100.0 + 1.0);
    // Exactly hero's row at the node after OOP's check: its frequencies, and its EVs over the real big blind (in f64,
    // as assembly converts every EV, `assemble::final_from_solution`).
    let solution = support::surrogate_solution(&case);
    let node = solution.nodes.iter().find(|n| n.path == [Action::Check]).expect("the check child is exported");
    assert_eq!(node.actor, "ip");
    let hero = state.hero_cards.unwrap();
    let c = usize::from(combo_index(hero[0], hero[1]));
    let expected: Vec<ActionAdvice> = node.actions.iter().enumerate()
        .map(|(a, action)| ActionAdvice { action: *action, frequency: Some(node.probs[c][a]), ev_bb: Some((f64::from(node.ev_chips[c][a]) / 100.0) as f32),
            unavailable: None, headline: false })
        .collect();
    assert!(expected.iter().any(|a| a.ev_bb.is_some_and(|ev| ev > 1.0)), "the row's EVs are over 100 chips, so dividing by 2 would show");
    assert_eq!(experimental.actions, expected);
    // a different hero combo in the same public spot yields a different advice row
    let other = support::three_way_flop_ip_hero_bb_100_other_combo();
    let other_run = support::run_three_way_flop_script(&other);
    let other_last = final_of(&other_run.events);
    assert_ne!(other_last.experimental.as_ref().unwrap().actions, experimental.actions);
    // the public ranges are identical: hero cards never condition them
    assert_eq!(other_last.experimental.as_ref().unwrap().ranges_used, experimental.ranges_used);
    assert_eq!((run.solves[0].oop_range.clone(), run.solves[0].ip_range.clone()), (other_run.solves[0].oop_range.clone(), other_run.solves[0].ip_range.clone()));
}

/// The worker request of the surrogate is the synthetic root over the street-root public ranges, with the request's
/// real rake, target and deadline, built by the solve client's own request builder (id, spot, memory limit, background).
#[test]
fn the_surrogate_request_is_the_synthetic_root_over_the_street_root_public_ranges() {
    let state = support::three_way_flop();
    let case = support::surrogate_case(&state, &support::three_way_ranges());
    let run = support::run_three_way_flop_script(&state);
    assert_eq!(run.solves.len(), 1, "one solve: the surrogate's");
    let req = &run.solves[0];
    assert_eq!((req.board.clone(), req.pot, req.stack_oop, req.stack_ip, req.history.clone()), (state.board.clone(), 300, 400, 400, vec![]));
    assert_eq!((req.oop_range.clone(), req.ip_range.clone()), (case.ranges[0].clone(), case.ranges[1].clone()), "street-root public ranges, OOP then IP");
    let hero = state.hero_cards.unwrap();
    assert!(req.oop_range.0[usize::from(combo_index(hero[0], hero[1]))] > 0.0, "hero's own combo stays in hero's public range: never hero-conditioned");
    assert_eq!(req.tree, case.tree, "the street's template at the synthetic root");
    assert_eq!((req.rake_rate, req.rake_cap_mchips), (0.05, 5_000), "the hand's real rake");
    assert_eq!((req.target_bp, req.background, req.memory_limit_bytes), (50, false, engine::core::DEFAULT_MEMORY_LIMIT_BYTES));
    // Admitted and served at 0 ms on the fake clock: the whole flop budget to the street deadline, less the margins.
    let deadlines = Deadlines::for_request(0, Street::Flop, 10);
    assert_eq!((req.deadline_ms, req.extraction_margin_ms), ((deadlines.street_deadline_ms - 150) as u32, 600));
    assert_eq!(req.spot, engine::solve::spot_identity(req));
    assert_eq!((run.kills, run.restarts), (0, 0));
}

/// Spec 6: when the highest-equity opponent is all-in the surrogate is skipped entirely; the next-best seat is never
/// substituted, nothing is sent to the worker, and the multiway `Final` is as it was with a note naming why.
#[test]
fn an_all_in_highest_equity_opponent_skips_the_surrogate_without_substitution() {
    let state = support::three_way_flop_with_all_in_opponent();
    // UTG (all-in) holds KK, a set on Kh 7d 2c: the highest equity against hero's QQ, above the button's AA.
    let mut ranges = support::three_way_ranges();
    ranges.iter_mut().find(|(s, _)| *s == Seat(3)).unwrap().1 = core_ranges::parse_range("KK").unwrap();
    let roots = support::street_root_public_ranges_of(&state, &ranges);
    let cancel = AtomicBool::new(false);
    assert_eq!(engine::experimental::choose_opponent(state.hero, &roots.hero, &roots.others, &state.board, Duration::from_millis(500), &cancel), Some(Seat(3)));
    let run = support::run_three_way(&state, SeatRanges { ranges, on_ask: None }, vec![], ServeSeams::default());
    let last = run.final_rec();
    assert!(last.experimental.is_none(), "the button is not substituted");
    assert!(run.solves.is_empty(), "nothing reaches the worker");
    let notes = absent_notes(last);
    assert_eq!(notes.len(), 1, "{:?}", last.assumptions.notes);
    assert!(notes[0].contains("seat 3") && notes[0].contains("all-in"), "{notes:?}");
    let mut main = last.clone();
    main.assumptions.notes.retain(|n| !n.starts_with(engine::experimental::ABSENT_NOTE));
    assert_eq!(main, multiway_final_before_task_11(&state, &run.id, 3));
}

/// A surrogate whose solve fails, or hangs to its bound, leaves the multiway `Final` exactly as before this task with a
/// note naming why the block is absent; a hung worker is killed (the next solve relaunches it), never restarted here.
#[test]
fn a_failed_or_hung_surrogate_leaves_the_multiway_final_as_before_with_a_note() {
    let state = support::three_way_flop();
    let case = support::surrogate_case(&state, &support::three_way_ranges());
    for (status, cause, kills) in [("error", "internal", 0), ("hang", "no terminal result by the worker deadline", 1)] {
        let run = support::run_three_way(&state, SeatRanges::three_way(), support::surrogate_script(&case, status), ServeSeams::default());
        let last = run.final_rec();
        assert!(last.experimental.is_none(), "{status}");
        let notes = absent_notes(last);
        assert!(notes.len() == 1 && notes[0].contains(cause), "{status}: {:?}", last.assumptions.notes);
        let mut main = last.clone();
        main.assumptions.notes.retain(|n| !n.starts_with(engine::experimental::ABSENT_NOTE));
        assert_eq!(main, multiway_final_before_task_11(&state, &run.id, 3), "{status}");
        assert_eq!((run.solves.len(), run.kills, run.restarts), (1, kills, 0), "{status}: one solve, never retried");
        assert_eq!((run.snapshots, run.misses, run.cache_entries), (0, 0, 0), "{status}");
    }
}

/// A watchdog fire after the surrogate answered and before the engine claims its `Final` (spec 7): the watchdog
/// delivers the multiway `Final` the engine was about to deliver, block included (the retained payload), never the
/// `DeadlineExceeded` fallback.
#[test]
fn a_watchdog_fire_before_the_claim_delivers_the_multiway_final_with_its_block() {
    let state = support::three_way_flop();
    let case = support::surrogate_case(&state, &support::three_way_ranges());
    let mut rig = FlopRig::new(support::surrogate_script(&case, "ok"));
    *rig.core.range_source.lock().unwrap() = Box::new(SeatRanges::three_way());
    let fire_ms = Deadlines::for_request(0, Street::Flop, 10).watchdog_fire_ms();
    let (clock, ended) = (rig.clock.clone(), rig.core.watchdog.ended_threads());
    let seams = ServeSeams { before_claim: Some(Arc::new(move || {
        clock.set_ms(fire_ms);
        ended.wait_for(1);
    })), ..ServeSeams::default() };
    let served = rig.serve_with(&state, seams);
    rig.core.shutdown();
    let last = served.final_rec();
    assert!(last.experimental.is_some(), "the retained payload carries the block: {:?}", last.assumptions.notes);
    assert!(matches!(last.coverage, Coverage::Unsupported { reason: UnsupportedReason::MultiwayEv { pot_eligible: 3 }, .. }));
    assert!(absent_notes(last).is_empty());
    let record = rig.records().pop().expect("the watchdog's Final is logged");
    assert!(record.final_violation, "delivered by the watchdog");
}

/// A watchdog fire before the surrogate has answered (here: while the range source is read): the watchdog delivers the
/// multiway `Final` without the block, with the note that it was not ready, never the `DeadlineExceeded` fallback; the
/// surrogate then has no time left and sends nothing.
#[test]
fn a_watchdog_fire_before_the_surrogate_answers_delivers_the_multiway_final_without_it() {
    let state = support::three_way_flop();
    let mut rig = FlopRig::new(vec![]);
    let fire_ms = Deadlines::for_request(0, Street::Flop, 10).watchdog_fire_ms();
    let (clock, ended) = (rig.clock.clone(), rig.core.watchdog.ended_threads());
    let stall: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        clock.set_ms(fire_ms);
        ended.wait_for(1);
    });
    *rig.core.range_source.lock().unwrap() = Box::new(SeatRanges { ranges: support::three_way_ranges(), on_ask: Some(stall) });
    let served = rig.serve(&state);
    let solves = rig.solves();
    rig.core.shutdown();
    let last = served.final_rec();
    assert!(last.experimental.is_none());
    assert!(matches!(last.coverage, Coverage::Unsupported { reason: UnsupportedReason::MultiwayEv { pot_eligible: 3 }, .. }));
    let notes = absent_notes(last);
    assert!(notes.len() == 1 && notes[0].contains("final delivery"), "{:?}", last.assumptions.notes);
    assert!(solves.is_empty(), "no time was left for the surrogate's solve");
}

/// The replay's range source answers every seat's published street-root public range (board-blocked, never
/// hero-conditioned) for the surrogate; a heads-up source of one OOP and one IP range answers none.
#[test]
fn the_replay_range_source_answers_every_seats_street_root_public_range() {
    let state = support::three_way_flop();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let store = Arc::new(core_preflop::PreflopStore::from_sources(vec![]));
    let source = engine::replay_bridge::ReplayRanges { store: store.clone(), snapshots: Arc::new(Mutex::new(engine::snapshots::SnapshotStore::new())), identity: identity.clone() };
    let seats = [Seat(2), Seat(3), Seat(0)];
    assert!(source.seat_ranges(&state, &seats).is_err(), "no active decision: nothing is read");
    {
        let mut ids = identity.lock().unwrap();
        ids.set_config();
        ids.begin_hand();
        ids.next_decision().unwrap();
    }
    let answered = source.seat_ranges(&state, &seats).expect("the active decision of the state's hand");
    let replayed = core_replay::replay(core_replay::ReplayInput { cfg: &state.config, state: &state, store: &store, snapshots: &[], missing: &[] });
    let hero = state.hero_cards.unwrap();
    let hero_combo = usize::from(combo_index(hero[0], hero[1]));
    assert_eq!(answered.iter().map(|(s, _)| *s).collect::<Vec<_>>(), seats, "in the order asked");
    for (seat, range) in &answered {
        let mut expected = replayed.ranges[usize::from(seat.0)].clone().unwrap();
        core_ranges::block_public(&mut expected, &state.board);
        assert!(*range == expected, "seat {seat:?}: the replay's published range, board-blocked");
        assert!(range.0[hero_combo] > 0.0, "seat {seat:?}: hero's cards never condition a public range");
    }
    let explicit = ExplicitRanges { oop: Some(Range1326([1.0; 1326])), ip: Some(Range1326([1.0; 1326])) };
    assert!(explicit.seat_ranges(&state, &seats).is_err(), "one OOP and one IP range name no third seat");
}

/// Fix round 1, P4T11-I1 (ruling 11-I1; spec 7, ruling 28-I4): a request admitted and then superseded (here by a
/// re-request of the same hand) before `engine-main` serves it reads no ranges and runs no opponent equity: its active
/// identity is checked under the identity lock right after it installs its equity token, the token is set, and the
/// surrogate is skipped. Nothing reaches the worker and no event of the stale decision is emitted.
#[test]
fn a_request_superseded_before_it_is_served_reads_no_ranges_and_runs_no_equity() {
    let state = support::three_way_flop();
    let mut rig = FlopRig::new(vec![]);
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let on_ask: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    *rig.core.range_source.lock().unwrap() = Box::new(SeatRanges { ranges: support::three_way_ranges(), on_ask: Some(on_ask) });
    let estimates = Arc::new(AtomicUsize::new(0));
    let counted = estimates.clone();
    let opponent_equity: PairEquity = Arc::new(move |_opp: &Range1326, _hero: &Range1326, _board: &[Card], _allowance: Duration, _cancel: &AtomicBool| {
        counted.fetch_add(1, Ordering::SeqCst);
        Some((0.5, EquityMethod::Exact))
    });
    let a = rig.identity.lock().unwrap().next_decision().unwrap();
    let (sink, events) = RecordingSink::new(rig.clock.clone(), Some(rig.worker.clone()));
    let sink: SharedSink = Arc::new(Mutex::new(Box::new(sink)));
    let req = LiveRequest::admitted(&rig.core, a.clone(), state.clone(), rig.clock.now_ms(), sink);
    let b = rig.identity.lock().unwrap().next_decision().unwrap();
    assert!(b != a && b.hand_id == a.hand_id, "B re-requests the same hand");
    // Fix round 2, 11-N1: the token A installed is read in `finish` before its `Final` is claimed (the `before_claim`
    // seam), before the stale exit (`retire_stale`) and the teardown (`EngineCore::shutdown`) would set it anyway, so
    // only the guard can have set it by then.
    let (slot, seen_at_claim) = (rig.core.equity_cancel.clone(), Arc::new(AtomicBool::new(false)));
    let seen = seen_at_claim.clone();
    let before_claim: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let token = slot.lock().unwrap().clone().expect("A installed its equity token");
        seen.store(token.load(Ordering::SeqCst), Ordering::SeqCst);
    });
    serve_request_with(&mut rig.core, req, ServeSeams { equity: Some(support::stub_equity()), opponent_equity: Some(opponent_equity), before_claim: Some(before_claim),
        ..ServeSeams::default() });
    let solves = rig.solves();
    rig.core.shutdown();
    assert_eq!(asked.load(Ordering::SeqCst), 0, "the stale request read no street-root ranges");
    assert_eq!(estimates.load(Ordering::SeqCst), 0, "the stale request ran no opponent equity");
    assert!(solves.is_empty(), "nothing reaches the worker");
    assert!(events.lock().unwrap().is_empty(), "no event of the stale decision");
    assert!(seen_at_claim.load(Ordering::SeqCst), "the guard set the stale request's token before its Final was claimed");
}

/// Fix round 1, P4T11-M1 (ruling 11-M1): public ranges with no pairwise-compatible holdings (hero publicly only on AsAh,
/// every opponent only on combos holding the As) leave the block absent with a note that names that cause, never an
/// equity-phase overrun, and send nothing to the worker.
#[test]
fn incompatible_public_ranges_are_never_reported_as_an_overrun() {
    let state = support::three_way_flop();
    let mut ranges = support::three_way_ranges();
    for (seat, text) in [(Seat(2), "AsAh"), (Seat(3), "AsKs,AsQs"), (Seat(0), "AsJs,AsTs")] {
        ranges.iter_mut().find(|(s, _)| *s == seat).unwrap().1 = core_ranges::parse_range(text).unwrap();
    }
    let run = support::run_three_way(&state, SeatRanges { ranges, on_ask: None }, vec![], ServeSeams::default());
    let last = run.final_rec();
    assert!(last.experimental.is_none());
    let notes = absent_notes(last);
    assert_eq!(notes.len(), 1, "{:?}", last.assumptions.notes);
    assert!(!notes[0].contains("overran") && !notes[0].contains("within its budget"), "no timeout is claimed: {notes:?}");
    assert!(notes[0].contains("compatible"), "the cause is named: {notes:?}");
    assert!(run.solves.is_empty(), "nothing reaches the worker");
}

/// `select_opponent` over the fake clock `clock` up to `cutoff_ms`, with an estimator that follows `script` (per call:
/// the fake milliseconds it spends, and its answer) and records the allowance each call was given.
#[allow(clippy::too_many_arguments)]
fn select_with(clock: &Arc<FakeClock>, cutoff_ms: u64, hero: Seat, hero_public: &Range1326, others: &[(Seat, Range1326)], board: &[Card], cancel: &AtomicBool,
    script: Vec<(u64, Option<f32>)>) -> (Result<Seat, NoOpponent>, Vec<u64>) {
    let script = Mutex::new(script.into_iter());
    let allowances = Mutex::new(Vec::new());
    let estimate = |_opp: &Range1326, _hero: &Range1326, _board: &[Card], allowance: Duration, _cancel: &AtomicBool| {
        allowances.lock().unwrap().push(u64::try_from(allowance.as_millis()).unwrap());
        let (spend, answer) = script.lock().unwrap().next().expect("an estimate the script did not expect was started");
        clock.advance_ms(spend);
        answer.map(|equity| (equity, EquityMethod::Exact))
    };
    let chosen = select_opponent(clock.as_ref(), cutoff_ms, hero, hero_public, others, board, cancel, &estimate);
    (chosen, allowances.into_inner().unwrap())
}

/// Fix round 1, P4T11-I2 (ruling 11-I2; spec 6 "skipped ... when the equity phase overran", spec 7 "every phase
/// receives only the remaining time"): opponent selection keeps one absolute cutoff on the engine clock. The clock is
/// read again before each candidate, each estimate gets at most the lesser of its share and what is left, no estimate
/// starts once the cutoff has come, and a selection that finishes after it is refused. The skip causes stay apart
/// (P4T11-M1): no pairwise-compatible holdings, no estimate computed, cancelled, no other seat.
#[test]
fn opponent_selection_keeps_one_absolute_cutoff() {
    let state = support::three_way_flop();
    let roots = support::street_root_public_ranges(&state);
    assert_eq!(roots.others.iter().map(|(s, _)| *s).collect::<Vec<_>>(), [Seat(3), Seat(0)], "UTG, then the button");
    let live = AtomicBool::new(false);
    let select = |cutoff_ms: u64, script: Vec<(u64, Option<f32>)>| {
        let clock = FakeClock::new();
        select_with(&clock, cutoff_ms, state.hero, &roots.hero, &roots.others, &state.board, &live, script)
    };
    // Two 250 ms shares of a 500 ms phase; the highest equity wins.
    assert_eq!(select(500, vec![(100, Some(0.2)), (100, Some(0.8))]), (Ok(Seat(0)), vec![250, 250]));
    // The first estimate spends 400 ms: the second gets only the 100 ms left, not a fresh 250 ms share.
    assert_eq!(select(500, vec![(400, Some(0.8)), (50, Some(0.2))]), (Ok(Seat(3)), vec![250, 100]));
    // The first estimate spends the whole phase: the second is never started.
    assert_eq!(select(500, vec![(500, Some(0.8))]), (Err(NoOpponent::Overran), vec![250]));
    // The last estimate finishes after the cutoff: the selection is refused, even with an opponent found.
    assert_eq!(select(500, vec![(100, Some(0.2)), (401, Some(0.8))]), (Err(NoOpponent::Overran), vec![250, 250]));
    // The cutoff has already come: nothing is started.
    let late = FakeClock::new();
    late.set_ms(600);
    assert_eq!(select_with(&late, 500, state.hero, &roots.hero, &roots.others, &state.board, &live, vec![]), (Err(NoOpponent::Overran), vec![]));
    // Every estimate ends without a value inside the phase: not an overrun.
    assert_eq!(select(500, vec![(10, None), (10, None)]), (Err(NoOpponent::NotComputed), vec![250, 250]));
    // Cancelled (the request superseded): nothing is started.
    let clock = FakeClock::new();
    assert_eq!(select_with(&clock, 500, state.hero, &roots.hero, &roots.others, &state.board, &AtomicBool::new(true), vec![]), (Err(NoOpponent::Cancelled), vec![]));
    // No other seat.
    assert_eq!(select_with(&clock, 500, state.hero, &roots.hero, &[], &state.board, &live, vec![]), (Err(NoOpponent::NoCandidates), vec![]));
    // No pairwise-compatible holdings (hero only on AsAh, the others only on combos holding the As): no estimate, and
    // that cause, never an overrun. One compatible seat among incompatible ones is given the whole phase.
    let only = |text: &str| core_ranges::parse_range(text).unwrap();
    let incompatible = vec![(Seat(3), only("AsKs,AsQs")), (Seat(0), only("AsJs,AsTs"))];
    assert_eq!(select_with(&clock, 500, state.hero, &only("AsAh"), &incompatible, &state.board, &live, vec![]), (Err(NoOpponent::Incompatible), vec![]));
    let mixed = vec![(Seat(3), only("AsKs")), (Seat(0), only("QcQd"))];
    assert_eq!(select_with(&clock, 500, state.hero, &only("AsAh"), &mixed, &state.board, &live, vec![(0, Some(0.1))]), (Ok(Seat(0)), vec![500]));
    // Each cause's note: only an overrun claims one.
    assert!(NoOpponent::Overran.why().contains("overran"));
    for cause in [NoOpponent::NoCandidates, NoOpponent::Cancelled, NoOpponent::Incompatible, NoOpponent::NotComputed] {
        assert!(!cause.why().contains("overran"), "{cause:?}: {}", cause.why());
    }
    assert_eq!(NoOpponent::NotComputed.why(), "no opponent had computable pairwise-compatible equity within the budget");
}

/// Fix round 1, P4T11-I2 at the request (ruling 11-I2): the multiway arm's opponent selection has one cutoff, 500 ms
/// after the equity phase starts on the engine clock (here 0 ms). An earlier candidate that spends it means no later
/// estimate is started; a last candidate that finishes after it refuses the selection. Either way no surrogate solve
/// is sent, and the note names the overrun.
#[test]
fn a_candidate_that_spends_the_equity_cutoff_stops_selection_and_the_surrogate() {
    let state = support::three_way_flop();
    for (spends, started) in [(vec![500u64], vec![500u64 / 2]), (vec![0, 501], vec![250, 250])] {
        let mut rig = FlopRig::new(vec![]);
        *rig.core.range_source.lock().unwrap() = Box::new(SeatRanges::three_way());
        let allowances = Arc::new(Mutex::new(Vec::new()));
        let (clock, log) = (rig.clock.clone(), allowances.clone());
        let script = spends.clone();
        let opponent_equity: PairEquity = Arc::new(move |_opp: &Range1326, _hero: &Range1326, _board: &[Card], allowance: Duration, _cancel: &AtomicBool| {
            let mut log = log.lock().unwrap();
            let spend = *script.get(log.len()).expect("an estimate the script did not expect was started");
            log.push(u64::try_from(allowance.as_millis()).unwrap());
            clock.advance_ms(spend);
            Some((0.5, EquityMethod::Exact))
        });
        let served = rig.serve_with(&state, ServeSeams { opponent_equity: Some(opponent_equity), ..ServeSeams::default() });
        let solves = rig.solves();
        rig.core.shutdown();
        assert_eq!(*allowances.lock().unwrap(), started, "{spends:?}: the estimates started and their allowances");
        assert!(solves.is_empty(), "{spends:?}: no surrogate solve");
        let last = served.final_rec();
        assert!(last.experimental.is_none() && matches!(last.coverage, Coverage::Unsupported { reason: UnsupportedReason::MultiwayEv { pot_eligible: 3 }, .. }));
        let notes = absent_notes(last);
        assert!(notes.len() == 1 && notes[0].contains("overran"), "{spends:?}: {:?}", last.assumptions.notes);
    }
}
