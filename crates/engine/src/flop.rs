//! The flop template policy of spec 10.1 (plan 4 Task 9): which template a live flop decision is solved with, from
//! the preflop history and the V3 measurement of `flop_min_v1`.
//!
//! Three or more preflop wagers (a 3-bet or larger pot) always use `flop_fast_v1`. A single-raised pot (exactly two
//! wagers: the live big blind or straddle, then one raise) is solved live with `flop_min_v1` only when V3 admitted it,
//! its measured p95 at most 10 s at both 100bb and 200bb, and with `flop_fast_v1` to the deadline otherwise. A limped
//! pot (one wager, no voluntary raise) is not a pre-solver single-raised-pot scenario and is served with the
//! conservative `flop_fast_v1`. The pot class is derived from the history, never from the ranges.
//!
//! V3 admission is evidence, not configuration: `load_v3_policy` admits `flop_min_v1` only from a measurement record
//! whose whole provenance (template signature, solver commit, adapter and rules versions, storage-mode policy, source
//! lock, machine) matches the one expected; anything else keeps `FlopPolicy::from_v3(None, None)`, the conservative
//! policy every `EngineCore` starts with. There is no table of unqualified timings.

use proto::{Action, HandState, Street, TakenAction};

/// Whether V3 admitted `flop_min_v1` as the live single-raised-pot template (`EngineCore::flop_policy`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlopPolicy {
    pub min_admitted: bool,
}

impl FlopPolicy {
    /// §10.1: three or more preflop wagers always use `flop_fast_v1`; a single-raised pot uses `flop_min_v1` only when
    /// V3 admitted it at both depths. A limped pot (one wager) is not a single-raised pot and uses `flop_fast_v1`.
    pub fn live_template(&self, preflop_wagers: u32) -> &'static str {
        if preflop_wagers == 2 && self.min_admitted {
            "flop_min_v1"
        } else {
            "flop_fast_v1"
        }
    }

    /// V3: `flop_min_v1` is admitted iff its measured p95 is at most 10 s at 100bb and 200bb.
    pub fn from_v3(p95_100: Option<f64>, p95_200: Option<f64>) -> Self {
        Self { min_admitted: [p95_100, p95_200].iter().all(|v| v.is_some_and(|x| x.is_finite() && x > 0.0 && x <= 10.0)) }
    }
}

/// §10.1 classification from history, never from ranges. The live BB (or straddle) is the first wager; each voluntary
/// action that raises the facing amount adds one. Forced posts are not counted twice and calls/checks/folds never
/// increment.
pub fn preflop_wagers(state: &HandState) -> u32 {
    let mut wagers = 1;
    let mut facing = state.config.straddle.as_ref().map_or(state.config.bb_chips, |s| s.amount_chips);
    for TakenAction { street, action, .. } in &state.actions {
        if *street != Street::Preflop {
            break;
        }
        if let Action::Raise { to } | Action::Bet { to } | Action::AllIn { to } = action {
            if *to > facing {
                facing = *to;
                wagers += 1;
            }
        }
    }
    wagers
}

/// A limped pot (no voluntary raise) is not a pre-solver SRP scenario and is served with the conservative fast
/// template.
pub fn is_limped(state: &HandState) -> bool {
    preflop_wagers(state) == 1
}

/// The provenance-checked V3 measurement of `flop_min_v1`: the exact template signature, solver commit, adapter and
/// rules versions, storage-mode policy, source-lock hash and machine it was measured with, and its p95 in seconds at
/// 100bb and 200bb. Plan 4 Task 24 serializes the matching record alongside its raw V3 measurements.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct V3PolicyEvidence {
    pub template_signature: String,
    pub solver_commit: String,
    pub adapter_version: u16,
    pub rules_version: u16,
    pub storage_mode_policy: String,
    pub source_lock_sha256: String,
    pub machine_id: String,
    pub p95_100bb_s: f64,
    pub p95_200bb_s: f64,
}

/// The policy the V3 evidence at `path` supports: `FlopPolicy::from_v3(Some(p95_100), Some(p95_200))` from the file's
/// own measurements when every provenance field matches `expected` (whose p95 fields are not compared), and the
/// conservative `FlopPolicy::from_v3(None, None)` for a missing, unreadable or mismatched record.
pub fn load_v3_policy(path: &std::path::Path, expected: &V3PolicyEvidence) -> FlopPolicy {
    let Ok(bytes) = std::fs::read(path) else { return FlopPolicy::from_v3(None, None) };
    let Ok(e) = serde_json::from_slice::<V3PolicyEvidence>(&bytes) else { return FlopPolicy::from_v3(None, None) };
    let provenance_matches = e.template_signature == expected.template_signature
        && e.solver_commit == expected.solver_commit
        && e.adapter_version == expected.adapter_version
        && e.rules_version == expected.rules_version
        && e.storage_mode_policy == expected.storage_mode_policy
        && e.source_lock_sha256 == expected.source_lock_sha256
        && e.machine_id == expected.machine_id;
    if provenance_matches {
        FlopPolicy::from_v3(Some(e.p95_100bb_s), Some(e.p95_200bb_s))
    } else {
        FlopPolicy::from_v3(None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{board, cfg_1_2, hand, play};
    use proto::{Seat, UtgStraddle};
    use std::path::PathBuf;
    use proto::Action::{AllIn, Bet, Call, Check, Fold, Raise};

    const NOT_ADMITTED: FlopPolicy = FlopPolicy { min_admitted: false };
    const ADMITTED: FlopPolicy = FlopPolicy { min_admitted: true };

    /// Six seats of `stack` chips, the button on seat 0 (SB 1, BB 2, UTG 3, HJ 4, CO 5), blinds 5/10, hero the BB.
    fn preflop(stack: u32, actions: &[Action]) -> HandState {
        play(&hand(&(0..6).map(|i| (Seat(i), stack)).collect::<Vec<_>>(), Seat(0), Seat(2), None), actions)
    }

    /// The same table with a 20-chip UTG straddle (preflop order HJ, CO, BTN, SB, BB, UTG).
    fn straddled(stacks: [u32; 6], actions: &[Action]) -> HandState {
        let (_, mut hc) = cfg_1_2();
        hc.straddle = Some(UtgStraddle { amount_chips: 20 });
        let s = core_model::begin_hand(&hc, core_model::BeginHand { hand_id: 1, button: Seat(0), hero: Seat(2), hero_cards: None, dealt: (0..6).map(Seat).collect(),
            stacks_start: stacks.to_vec() }).expect("a straddled six-handed hand");
        play(&s, actions)
    }

    /// Only a single-raised pot (exactly two wagers) may use an admitted `flop_min_v1`; a limped pot, a 3-bet or larger
    /// pot, and every pot while V3 has admitted nothing, use `flop_fast_v1`.
    #[test]
    fn the_live_template_is_flop_min_only_for_an_admitted_single_raised_pot() {
        assert_eq!(ADMITTED.live_template(2), "flop_min_v1");
        for wagers in [0, 1, 3, 4, 5, u32::MAX] {
            assert_eq!(ADMITTED.live_template(wagers), "flop_fast_v1", "{wagers} preflop wagers");
        }
        for wagers in [0, 1, 2, 3, 4, u32::MAX] {
            assert_eq!(NOT_ADMITTED.live_template(wagers), "flop_fast_v1", "{wagers} preflop wagers, nothing admitted");
        }
    }

    /// V3 criterion (c): admitted iff both measured p95s are finite, positive and at most 10 s; 10 s itself is admitted.
    #[test]
    fn v3_admits_flop_min_only_within_ten_seconds_at_both_depths() {
        assert_eq!(FlopPolicy::from_v3(Some(10.0), Some(10.0)), ADMITTED);
        assert_eq!(FlopPolicy::from_v3(Some(f64::MIN_POSITIVE), Some(9.999)), ADMITTED);
        let refused = [
            (None, None),
            (Some(5.0), None),
            (None, Some(5.0)),
            (Some(10.000_001), Some(5.0)),
            (Some(5.0), Some(10.000_001)),
            (Some(0.0), Some(5.0)),
            (Some(5.0), Some(-0.0)),
            (Some(-1.0), Some(5.0)),
            (Some(f64::NAN), Some(5.0)),
            (Some(5.0), Some(f64::INFINITY)),
            (Some(f64::NEG_INFINITY), Some(5.0)),
        ];
        for (p95_100, p95_200) in refused {
            assert_eq!(FlopPolicy::from_v3(p95_100, p95_200), NOT_ADMITTED, "p95 {p95_100:?} at 100bb, {p95_200:?} at 200bb");
        }
    }

    /// Legal histories (`core_model` built): the live big blind is the first wager and each raise one more; calls,
    /// checks and folds never count, nor does anything after the preflop.
    #[test]
    fn preflop_wagers_count_the_big_blind_and_each_raise() {
        let limped = preflop(1000, &[Call, Call, Call, Call, Call, Check]);
        assert_eq!((preflop_wagers(&limped), is_limped(&limped)), (1, true));
        let srp = preflop(1000, &[Fold, Fold, Fold, Raise { to: 25 }, Fold, Call]);
        assert_eq!((preflop_wagers(&srp), is_limped(&srp)), (2, false));
        let three_bet = preflop(1000, &[Fold, Fold, Fold, Raise { to: 25 }, Fold, Raise { to: 110 }, Call]);
        assert_eq!((preflop_wagers(&three_bet), is_limped(&three_bet)), (3, false));
        let four_bet_jam = preflop(1000, &[Fold, Fold, Fold, Raise { to: 25 }, Fold, Raise { to: 110 }, AllIn { to: 1000 }, Call]);
        assert_eq!(preflop_wagers(&four_bet_jam), 4);
        // a limp, then a raise over the limpers: two wagers
        let iso = preflop(1000, &[Call, Fold, Fold, Raise { to: 40 }, Fold, Fold, Call]);
        assert_eq!(preflop_wagers(&iso), 2);
        // a short all-in above the open is a wager, whether or not it is a full raise
        let short_jam = play(&hand(&[(Seat(0), 1000), (Seat(1), 50), (Seat(2), 1000), (Seat(3), 1000), (Seat(4), 1000), (Seat(5), 1000)], Seat(0), Seat(2), None),
            &[Fold, Fold, Fold, Raise { to: 45 }, AllIn { to: 50 }, Fold, Call]);
        assert_eq!(preflop_wagers(&short_jam), 3);
        // postflop bets and raises never count
        let flop = play(&board(&srp, "Kh 7d 2c"), &[Bet { to: 30 }, Raise { to: 90 }, Call]);
        assert_eq!(preflop_wagers(&flop), 2);
        let turn = play(&board(&play(&board(&three_bet, "Kh 7d 2c"), &[Check, Check]), "Kh 7d 2c 4d"), &[Bet { to: 100 }]);
        assert_eq!(preflop_wagers(&turn), 3);
    }

    /// With a straddle, the straddle is the first wager (its post is not counted again) and each raise over it one more.
    #[test]
    fn a_straddle_is_the_first_wager() {
        let limped = straddled([1000; 6], &[Call, Call, Call, Call, Call, Check]);
        assert_eq!((preflop_wagers(&limped), is_limped(&limped)), (1, true));
        let raised = straddled([1000; 6], &[Raise { to: 60 }, Fold, Fold, Fold, Fold, Call]);
        assert_eq!(preflop_wagers(&raised), 2);
        let three_bet = straddled([1000; 6], &[Raise { to: 60 }, Fold, Raise { to: 180 }, Fold, Fold, Fold, Fold]);
        assert_eq!(preflop_wagers(&three_bet), 3);
    }

    /// The history contract on synthetic histories (`core_model` itself records an all-in that does not exceed the amount
    /// faced as a `Call`, so no legal hand carries one): a recorded wager counts only when it raises the amount faced,
    /// which starts at the live big blind, or at the straddle when one is posted; forced posts never count twice.
    #[test]
    fn only_a_wager_that_raises_the_amount_faced_counts() {
        let with = |state: HandState, actions: &[Action]| -> HandState {
            let seat = Seat(3);
            HandState { actions: actions.iter().map(|&action| TakenAction { seat, street: Street::Preflop, action, paid: 0 }).collect(), ..state }
        };
        let plain = preflop(1000, &[]);
        assert_eq!(preflop_wagers(&with(plain.clone(), &[AllIn { to: 8 }])), 1, "an all-in for less than the big blind");
        assert_eq!(preflop_wagers(&with(plain.clone(), &[AllIn { to: 10 }])), 1, "an all-in for exactly the big blind");
        assert_eq!(preflop_wagers(&with(plain.clone(), &[Raise { to: 11 }])), 2, "one chip over the big blind");
        assert_eq!(preflop_wagers(&with(plain, &[Raise { to: 30 }, AllIn { to: 30 }, AllIn { to: 25 }, Raise { to: 31 }])), 3, "the amount faced only rises");
        let straddle = straddled([1000; 6], &[]);
        assert_eq!(preflop_wagers(&with(straddle.clone(), &[AllIn { to: 15 }])), 1, "above the big blind but below the straddle");
        assert_eq!(preflop_wagers(&with(straddle.clone(), &[AllIn { to: 20 }])), 1, "exactly the straddle");
        assert_eq!(preflop_wagers(&with(straddle, &[AllIn { to: 21 }])), 2);
    }

    fn evidence() -> V3PolicyEvidence {
        V3PolicyEvidence {
            template_signature: "flop_min_v1/synthetic-signature".into(),
            solver_commit: "9d1509fe5077d019825f833eed04b16d342dfda1".into(),
            adapter_version: 1,
            rules_version: 3,
            storage_mode_policy: "f32<=2GiB;i16<=8GiB".into(),
            source_lock_sha256: "ab".repeat(32),
            machine_id: "synthetic-machine".into(),
            p95_100bb_s: 8.5,
            p95_200bb_s: 9.75,
        }
    }

    /// A file of its own under the temporary directory, removed on drop.
    struct TempFile(PathBuf);
    impl TempFile {
        fn with(name: &str, bytes: &[u8]) -> TempFile {
            let path = std::env::temp_dir().join(format!("pokerai-v3-policy-{name}-{}.json", std::process::id()));
            std::fs::write(&path, bytes).unwrap();
            TempFile(path)
        }
    }
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Matching provenance admits `flop_min_v1` from the file's own measurements (the expected record's p95 fields are
    /// not provenance); any one provenance field that differs keeps the conservative policy.
    #[test]
    fn v3_evidence_admits_flop_min_only_when_every_provenance_field_matches() {
        let e = evidence();
        let file = TempFile::with("matching", &serde_json::to_vec(&e).unwrap());
        assert_eq!(load_v3_policy(&file.0, &e), ADMITTED);
        assert_eq!(load_v3_policy(&file.0, &V3PolicyEvidence { p95_100bb_s: 99.0, p95_200bb_s: f64::NAN, ..e.clone() }), ADMITTED);
        let differing = [
            ("template_signature", V3PolicyEvidence { template_signature: "flop_min_v1/other".into(), ..e.clone() }),
            ("solver_commit", V3PolicyEvidence { solver_commit: "0000000000000000000000000000000000000000".into(), ..e.clone() }),
            ("adapter_version", V3PolicyEvidence { adapter_version: 2, ..e.clone() }),
            ("rules_version", V3PolicyEvidence { rules_version: 2, ..e.clone() }),
            ("storage_mode_policy", V3PolicyEvidence { storage_mode_policy: "f32 only".into(), ..e.clone() }),
            ("source_lock_sha256", V3PolicyEvidence { source_lock_sha256: "cd".repeat(32), ..e.clone() }),
            ("machine_id", V3PolicyEvidence { machine_id: "another-machine".into(), ..e.clone() }),
        ];
        for (field, expected) in differing {
            assert_eq!(load_v3_policy(&file.0, &expected), NOT_ADMITTED, "{field} differs");
        }
    }

    /// No evidence file, an unreadable or incomplete record, or matching evidence whose measurement misses 10 s at either
    /// depth, keeps the conservative policy; a matching measurement of exactly 10 s at both depths is admitted.
    #[test]
    fn v3_evidence_that_is_missing_malformed_or_too_slow_keeps_the_conservative_policy() {
        let e = evidence();
        let missing = std::env::temp_dir().join(format!("pokerai-v3-policy-absent-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&missing);
        assert_eq!(load_v3_policy(&missing, &e), NOT_ADMITTED, "no evidence file");
        assert_eq!(load_v3_policy(&TempFile::with("truncated", b"{\"template_signature\":").0, &e), NOT_ADMITTED, "malformed JSON");
        let mut incomplete = serde_json::to_value(&e).unwrap();
        incomplete.as_object_mut().unwrap().remove("machine_id");
        assert_eq!(load_v3_policy(&TempFile::with("incomplete", &serde_json::to_vec(&incomplete).unwrap()).0, &e), NOT_ADMITTED, "a record without its machine");
        let slow = V3PolicyEvidence { p95_200bb_s: 10.5, ..e.clone() };
        assert_eq!(load_v3_policy(&TempFile::with("slow", &serde_json::to_vec(&slow).unwrap()).0, &e), NOT_ADMITTED, "200bb p95 over 10 s");
        let at_bound = V3PolicyEvidence { p95_100bb_s: 10.0, p95_200bb_s: 10.0, ..e.clone() };
        assert_eq!(load_v3_policy(&TempFile::with("at-bound", &serde_json::to_vec(&at_bound).unwrap()).0, &e), ADMITTED, "10 s at both depths");
    }

    /// The record round-trips through its JSON form unchanged.
    #[test]
    fn v3_evidence_round_trips_through_json() {
        let e = evidence();
        let back: V3PolicyEvidence = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }
}
