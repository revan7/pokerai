//! P3.T8 -- prefix reconstruction and depth/rake/virtual-role selection (spec section 8.3; the
//! section 13.1 rows `depth_bucket_labels_per_prefix`, `straddle_mapping_labels`,
//! `rake_profile_ordering` and `pokerdata_action_path_lookup`).
//!
//! Every mapping rule is asserted twice: once on the pure function that states it, and once
//! through `PreflopStore::query` on a real `HandState` built by Plan 1's `begin_hand` and
//! `apply_action`, because the labels section 13.1 requires are the ones a caller actually sees.

use core_model::state::BeginHand;
use core_preflop::{
    load_bundle, node_key, BundleInfo, PreflopNode, PreflopNodeKey, PreflopSource, PreflopStep, PreflopStore,
    RakeProfile, SourceKind,
};
use proto::{
    Action, ApproxReason, Card, Derived, HandConfig, HandPhase, HandState, Position, Rake, Seat, Street,
    UnsupportedReason, UtgStraddle,
};
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("../../../fixtures/preflop/synthetic_v2/manifest.json");

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/preflop/synthetic_v2")
}

/// The committed synthetic PokerData bundle. A missing or unreadable fixture fails the test
/// (standing ruling (e)), it is never skipped.
fn synthetic_bundle() -> Box<dyn PreflopSource> {
    let dir = fixture_dir();
    load_bundle(&dir.join("manifest.json"), &dir.join("nodes.json")).expect("the committed synthetic_v2 bundle loads")
}

fn synthetic_store() -> PreflopStore {
    PreflopStore::from_sources(vec![synthetic_bundle()])
}

/// A stand-in bundle whose `BundleInfo` is the synthetic manifest with `mutate` applied and whose
/// every lookup misses. It is what proves *which* candidate `query` selected: a miss in the
/// selected source is reported as `MissingPreflopNode` and never hidden by searching a
/// lower-ranked bundle.
struct EmptyBundle {
    info: BundleInfo,
}

impl PreflopSource for EmptyBundle {
    fn bundle_info(&self) -> &BundleInfo {
        &self.info
    }
    fn lookup(&self, _key: &PreflopNodeKey) -> Option<PreflopNode> {
        None
    }
    /// P3.T7 fix round 1, R1: `PreflopSource::nodes_have_no_ev` no longer has a default body,
    /// so this test-only double states its own answer explicitly -- vacuously `true`, since it
    /// never stores any node at all.
    fn nodes_have_no_ev(&self) -> bool {
        true
    }
}

fn variant(bundle_id: &str, mutate: impl FnOnce(&mut BundleInfo)) -> Box<dyn PreflopSource> {
    let mut info: BundleInfo = serde_json::from_str(MANIFEST).expect("the committed manifest deserializes");
    info.bundle_id = bundle_id.to_string();
    mutate(&mut info);
    Box::new(EmptyBundle { info })
}

fn config(sb: u32, bb: u32, straddle: Option<u32>, rake: Rake) -> HandConfig {
    HandConfig {
        config_revision: 7,
        sb_chips: sb,
        bb_chips: bb,
        straddle: straddle.map(|amount_chips| UtgStraddle { amount_chips }),
        rake,
        chip_label: "$1".into(),
    }
}

/// The fixture's own profile (5%, cap 0.5 bb, no-flop-no-drop) expressed in chips at `unit` chips
/// per source unit, so `rake_reason` sees an exact match and emits nothing. `unit * 500` is
/// `0.5 * unit` chips in thousandths, exact for an odd unit too.
fn exact_rake(unit: u32) -> Rake {
    Rake::PotRake { rate: 0.05, cap_mchips: unit * 500, no_flop_no_drop: true }
}

/// A copy of the committed bundle under a temporary directory, with `depths` replaced.
fn temp_bundle(case: &str, depths: &[u16]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("core_preflop_p3t8_{case}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("synthetic")).expect("temp bundle dir");
    let mut manifest: serde_json::Value = serde_json::from_str(MANIFEST).expect("the committed manifest deserializes");
    manifest["depths"] = serde_json::json!(depths);
    std::fs::write(dir.join("synthetic/manifest.json"), serde_json::to_string(&manifest).unwrap()).expect("manifest");
    std::fs::copy(fixture_dir().join("nodes.json"), dir.join("synthetic/nodes.json")).expect("nodes");
    dir
}

fn dealt_seats(n: usize) -> Vec<Seat> {
    (0..n as u8).map(Seat).collect()
}

/// Seat `n - 1` holds the button, so seat 0 is always the SB and the six-seat table's roles are
/// BTN 5, SB 0, BB 1, UTG 2, HJ 3, CO 4.
fn button_of(n: usize) -> Seat {
    Seat(n as u8 - 1)
}

fn table(cfg: &HandConfig, n: usize, stacks: &[u32]) -> HandState {
    core_model::begin_hand(
        cfg,
        BeginHand {
            hand_id: 1,
            button: button_of(n),
            hero: Seat(0),
            dealt: dealt_seats(n),
            stacks_start: stacks.to_vec(),
            hero_cards: None,
        },
    )
    .expect("the model admits this table")
}

fn act(state: &HandState, actions: &[Action]) -> HandState {
    let mut next = state.clone();
    for a in actions {
        next = core_model::apply_action(&next, *a).unwrap_or_else(|e| panic!("legal preflop action {a:?}: {e}"));
    }
    next
}

/// A `HandState` in the shape a deserialized one has -- the only way a format `core-model` refuses
/// to admit can reach a query at all, and exactly what `check_straddle` exists to reject.
fn raw_state(cfg: HandConfig, dealt: Vec<Seat>, stacks_start: Vec<u32>, button: Seat) -> HandState {
    HandState {
        hand_id: 1,
        hand_revision: 0,
        config: cfg,
        phase: HandPhase::Betting { street: Street::Preflop },
        button,
        hero: dealt[0],
        hero_cards: None,
        dealt,
        stacks_start,
        board: vec![],
        actions: vec![],
        derived: Derived::default(),
    }
}

/// A straddled hand at its first decision. `begin_hand` is used wherever the model admits the
/// table; the configurations it refuses -- a short post, a short-handed straddle -- fall back to
/// [`raw_state`].
fn straddled_state(sb: u32, bb: u32, s: u32, dealt: usize, stacks: u32) -> HandState {
    let cfg = config(sb, bb, Some(s), Rake::TimeCharge);
    let seats = dealt_seats(dealt);
    let stacks_start = vec![stacks; dealt];
    let begin = BeginHand {
        hand_id: 1,
        button: button_of(dealt),
        hero: Seat(0),
        dealt: seats.clone(),
        stacks_start: stacks_start.clone(),
        hero_cards: None,
    };
    core_model::begin_hand(&cfg, begin).unwrap_or_else(|_| raw_state(cfg, seats, stacks_start, button_of(dealt)))
}

#[test]
fn depth_bucket_labels_per_prefix() {
    use core_preflop::{asymmetric, bucket, depth_for, prominent_depth};
    let acquired = [20, 30, 40, 50, 70, 100, 150, 200];
    for (actual, used, prominent) in [(97., 100, false), (120., 100, true), (125., 150, true), (260., 200, true)] {
        assert_eq!(bucket(actual, &acquired), Some(used));
        assert_eq!(prominent_depth(actual, used), prominent);
    }
    assert_eq!(depth_for(100, &[104], 1), 100.0);
    // section 8.3: label AsymmetricStacks on any difference; 5% controls prominence only.
    assert_eq!(asymmetric(&[100.0, 104.0], 100), (true, false));
    assert_eq!(asymmetric(&[100.0, 110.0], 100), (true, true));
    assert_eq!(asymmetric(&[100.0, 100.0], 100), (false, false));

    // The 100-versus-104 case through `query`: the label is emitted, it is not prominent, and the
    // answer can never contribute `Exact` coverage.
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let mut stacks = vec![100 * unit; 6];
    stacks[3] = 104 * unit; // the HJ seat covers everyone by 4 bb
    let answer = synthetic_store().query(&cfg, &table(&cfg, 6, &stacks), 0);
    assert_eq!(answer.unsupported, None, "{answer:?}");
    assert_eq!(answer.actor, Some(Seat(2)), "UTG opens the orbit");
    assert!(answer.node.is_some(), "the depth-100 UTG RFI node is present: {answer:?}");
    assert!(
        answer.reasons.iter().any(
            |r| matches!(r, ApproxReason::AsymmetricStacks { stacks_bb, prominent: false } if stacks_bb.contains(&104.0))
        ),
        "{:?}",
        answer.reasons
    );
    assert!(
        !answer.reasons.iter().any(|r| matches!(r, ApproxReason::DepthBucket { .. })),
        "the actor's depth is exactly 100 bb: {:?}",
        answer.reasons
    );
    assert!(!answer.reasons.is_empty(), "an asymmetric table is never Exact");
}

#[test]
fn straddle_mapping_labels() {
    use proto::Position::*;
    for (physical, virtual_role) in [(Hj, Utg), (Co, Hj), (Btn, Co), (Sb, Btn), (Bb, Sb), (Utg, Bb)] {
        assert_eq!(core_preflop::virtual_position(physical, true), virtual_role);
        assert_eq!(core_preflop::virtual_position(physical, false), physical);
    }
    assert_eq!(core_preflop::normalized_posts(1, 2, 4), [0.25, 0.5, 1.0]);
    assert_eq!(core_preflop::normalized_posts(2, 5, 10), [0.2, 0.5, 1.0]);
    // section 8.3 rejects a short post, a short-handed straddle and a re-straddle before lookup.
    let short = straddled_state(/*sb*/ 1, /*bb*/ 2, /*S*/ 3, /*dealt*/ 6, /*stacks*/ 400); // S < 2 * bb
    assert!(matches!(core_preflop::check_straddle(&short), Err(core_model::RulesError::FormatUnsupported { .. })));
    let five_handed = straddled_state(1, 2, 4, 5, 400);
    assert!(matches!(
        core_preflop::check_straddle(&five_handed),
        Err(core_model::RulesError::FormatUnsupported { .. })
    ));
    let cannot_post = straddled_state(1, 2, 4, 6, 3); // stack below the full straddle
    assert!(matches!(core_preflop::check_straddle(&cannot_post),
        Err(core_model::RulesError::FormatUnsupported{detail})
        if detail=="straddler's starting stack does not cover the straddle"));
    // A re-straddle cannot be represented: `HandConfig.straddle` is `Option<UtgStraddle>`.
    assert!(serde_json::from_str::<proto::HandConfig>(
        r#"{"config_revision":0,"sb_chips":1,"bb_chips":2,
            "straddle":[{"amount_chips":4},{"amount_chips":8}],
            "rake":{"kind":"time_charge"},"chip_label":"$1"}"#
    )
    .is_err());
    // A legal straddle passes, and the query path reports it rather than refusing it.
    assert!(core_preflop::check_straddle(&straddled_state(1, 2, 4, 6, 400)).is_ok());
}

#[test]
fn rake_profile_ordering() {
    let mut source: core_preflop::BundleInfo = serde_json::from_str(MANIFEST).unwrap();
    let actual = proto::Rake::PotRake { rate: 0.10, cap_mchips: 6000, no_flop_no_drop: true };
    let rank = core_preflop::rake_rank(&actual, &source, 5);
    assert!((rank.1 - 0.7).abs() < 1e-6);
    assert!((rank.2 - 0.05).abs() < 1e-6);
    assert_eq!(rank.3, 0);
    let raked = core_preflop::rake_rank(&proto::Rake::TimeCharge, &source, 5);
    source.rake = Some(core_preflop::RakeProfile { rate: 0.0, cap_bb: 0.0, no_flop_no_drop: true });
    let free = core_preflop::rake_rank(&proto::Rake::TimeCharge, &source, 5);
    assert!(free.0 < raked.0);

    // section 13.1's row is about the bundle `query` selects, not only about the tuple: the
    // 5%-cap-0.5bb bundle wins over a farther cap and the mapping is reported.
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, Rake::PotRake { rate: 0.10, cap_mchips: 1_200_000, no_flop_no_drop: true });
    let state = table(&cfg, 6, &vec![100 * unit; 6]);
    let store = PreflopStore::from_sources(vec![
        synthetic_bundle(),
        variant("aa_far_cap", |i| {
            i.rake = Some(RakeProfile { rate: 0.10, cap_bb: 2.0, no_flop_no_drop: true });
            i.rake_profile = "10% cap 2bb".into();
        }),
    ]);
    let answer = store.query(&cfg, &state, 0);
    assert_eq!(answer.bundle.as_ref().map(|b| b.rake_profile.as_str()), Some("5% cap 0.5bb"), "{answer:?}");
    assert_eq!(answer.unsupported, None, "{answer:?}");
    assert!(
        answer
            .reasons
            .iter()
            .any(|r| matches!(r, ApproxReason::RakeProfileMapped { used, .. } if used == "5% cap 0.5bb")),
        "{:?}",
        answer.reasons
    );

    // A time charge takes an unraked bundle ahead of any raked one, even though only the raked
    // bundle holds the node: a miss in the selected source is never hidden.
    let timed = config(unit / 2, unit, None, Rake::TimeCharge);
    let store = PreflopStore::from_sources(vec![
        synthetic_bundle(),
        variant("zz_unraked", |i| i.rake = Some(RakeProfile { rate: 0.0, cap_bb: 0.0, no_flop_no_drop: true })),
    ]);
    let answer = store.query(&timed, &table(&timed, 6, &vec![100 * unit; 6]), 0);
    assert_eq!(answer.bundle.as_ref().map(|b| b.bundle_id.as_str()), Some("zz_unraked"), "{answer:?}");
    assert!(matches!(answer.unsupported, Some(UnsupportedReason::MissingPreflopNode { .. })), "{answer:?}");

    // An undocumented source rake always maps, with the fixed `used` string.
    let store = PreflopStore::from_sources(vec![variant("aa_chart", |i| i.rake = None)]);
    let answer = store.query(&cfg, &state, 0);
    assert!(
        answer.reasons.contains(&ApproxReason::RakeProfileMapped {
            actual: format!("{:?}", cfg.rake),
            used: "undocumented chart rake".into()
        }),
        "{:?}",
        answer.reasons
    );
}

#[test]
fn pokerdata_action_path_lookup() {
    use core_preflop::{PreflopNodeKey, PreflopStep::*};
    use proto::Position::*;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/preflop/synthetic_v2");
    let source = core_preflop::load_bundle(&dir.join("manifest.json"), &dir.join("nodes.json")).unwrap();
    let mut three = vec![(Utg, Raise { to_bb_x1000: 2500 }), (Hj, Raise { to_bb_x1000: 8750 })];
    three.extend([(Co, Fold), (Btn, Fold), (Sb, Fold), (Bb, Fold)]);
    let mut four = three.clone();
    four.push((Utg, Raise { to_bb_x1000: 22000 }));
    let squeeze = vec![(Utg, Raise { to_bb_x1000: 2500 }), (Hj, Call), (Co, Fold), (Btn, Fold), (Sb, Fold)];
    let sb = vec![(Utg, Fold), (Hj, Fold), (Co, Fold), (Btn, Fold)];
    let mut limp = sb.clone();
    limp.push((Sb, Call));
    for (history, actor) in [(vec![], Utg), (three, Utg), (four, Hj), (squeeze, Bb), (sb, Sb), (limp, Bb)] {
        let key = PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history };
        let node = source.lookup(&key).unwrap();
        assert_eq!(node.actor, actor);
        assert_eq!(node.probs.len(), 169);
        assert!(!node.actions.is_empty());
    }
    for history in [
        vec![(Utg, Call)],
        vec![(Utg, Raise { to_bb_x1000: 2500 }), (Hj, Raise { to_bb_x1000: 8750 }), (Co, Call)],
        vec![
            (Utg, Raise { to_bb_x1000: 2500 }),
            (Hj, Raise { to_bb_x1000: 8750 }),
            (Co, Fold),
            (Btn, Fold),
            (Sb, Call),
        ],
    ] {
        let key = PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history };
        assert!(source.lookup(&key).is_none());
    }

    // The same three absences through `query`, on histories the model itself produced: an absent
    // node is `MissingPreflopNode` carrying the key that was looked up, never a guessed node.
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let store = synthetic_store();
    let root = table(&cfg, 6, &vec![100 * unit; 6]);
    for actions in [
        vec![Action::Call],
        vec![Action::Raise { to: 2500 }, Action::Raise { to: 8750 }, Action::Call],
        vec![Action::Raise { to: 2500 }, Action::Raise { to: 8750 }, Action::Fold, Action::Fold, Action::Call],
    ] {
        let state = act(&root, &actions);
        let answer = store.query(&cfg, &state, actions.len());
        assert!(answer.node.is_none(), "{answer:?}");
        assert_eq!(
            answer.unsupported,
            Some(UnsupportedReason::MissingPreflopNode { key: answer.key.clone() }),
            "{answer:?}"
        );
    }

    // The present paths answer through `query` too, at their own prefix.
    let opened = act(&root, &[Action::Raise { to: 2500 }]);
    let answer = store.query(&cfg, &opened, 1);
    assert_eq!(answer.node.as_ref().map(|n| n.actor), Some(Hj), "{answer:?}");
    assert_eq!(answer.actor, Some(Seat(3)));
    assert_eq!(answer.unit, unit);
    assert!(answer.reasons.is_empty(), "an exact 100 bb symmetric table at the source's own rake: {answer:?}");
}

#[test]
fn query_at_a_prefix_ignores_later_actions() {
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let mut stacks = vec![100 * unit; 6];
    stacks[4] = 200 * unit; // the CO seat is the deep one, and folds later
    let store = synthetic_store();
    let early = act(&table(&cfg, 6, &stacks), &[Action::Raise { to: 2500 }]);
    let before = store.query(&cfg, &early, 1);
    let later = act(&early, &[Action::Raise { to: 8750 }, Action::Fold]);
    let after = store.query(&cfg, &later, 1);
    assert_eq!(before, after, "the answer at a prefix never depends on a later action");
    assert!(before.node.is_some(), "{before:?}");
    assert!(
        before.reasons.iter().any(|r| matches!(r, ApproxReason::AsymmetricStacks { prominent: true, .. })),
        "{:?}",
        before.reasons
    );
}

#[test]
fn short_handed_prefixes_are_lookup_only_folds() {
    use proto::Position::*;
    assert_eq!(core_preflop::short_handed_prefix(6), vec![]);
    assert_eq!(core_preflop::short_handed_prefix(5), vec![(Utg, PreflopStep::Fold)]);
    assert_eq!(core_preflop::short_handed_prefix(4), vec![(Utg, PreflopStep::Fold), (Hj, PreflopStep::Fold)]);
    assert_eq!(
        core_preflop::short_handed_prefix(3),
        vec![(Utg, PreflopStep::Fold), (Hj, PreflopStep::Fold), (Co, PreflopStep::Fold)]
    );

    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let store = synthetic_store();
    let full_folds = PreflopNodeKey {
        depth_bb: 100,
        rake_profile: "5% cap 0.5bb".into(),
        straddle: false,
        history: vec![
            (Utg, PreflopStep::Fold),
            (Hj, PreflopStep::Fold),
            (Co, PreflopStep::Fold),
            (Btn, PreflopStep::Fold),
        ],
    };
    // 3, 4 and 5 dealt seats all reach the six-max "folded to the SB" node once the physically
    // present seats have folded: the missing seats' folds exist only in the key.
    for (n, physical_folds) in [(3usize, 1usize), (4, 2), (5, 3)] {
        let state = act(&table(&cfg, n, &vec![100 * unit; n]), &vec![Action::Fold; physical_folds]);
        let answer = store.query(&cfg, &state, physical_folds);
        assert_eq!(answer.unsupported, None, "{n} dealt: {answer:?}");
        assert_eq!(answer.node.as_ref().map(|node| node.actor), Some(Sb), "{n} dealt: {answer:?}");
        assert_eq!(answer.actor, Some(Seat(0)), "{n} dealt: the SB seat is the physical actor");
        assert_eq!(answer.key, node_key(&full_folds), "{n} dealt: {answer:?}");
        assert!(answer.reasons.contains(&ApproxReason::ShortHandedMapped { dealt: n as u8 }), "{:?}", answer.reasons);
        assert_eq!(state.actions.len(), physical_folds, "the lookup-only folds never enter the hand state");
    }
    // A six-max table emits no mapping at all.
    let six = store.query(&cfg, &table(&cfg, 6, &vec![100 * unit; 6]), 0);
    assert!(!six.reasons.iter().any(|r| matches!(r, ApproxReason::ShortHandedMapped { .. })), "{:?}", six.reasons);
    // A short-handed prefix with no matching node stays missing.
    let five = store.query(&cfg, &table(&cfg, 5, &vec![100 * unit; 5]), 0);
    assert!(matches!(five.unsupported, Some(UnsupportedReason::MissingPreflopNode { .. })), "{five:?}");
    assert!(five.reasons.contains(&ApproxReason::ShortHandedMapped { dealt: 5 }), "{:?}", five.reasons);
}

/// `stacks_start` is aligned with `dealt`, while `Derived`'s vectors are indexed by `Seat.0`. On a
/// table whose dealt seats are not `0..n` the two orders differ, so indexing `stacks_start` by
/// `seat.0` would read another seat's stack -- a wrong depth bucket and a wrong `AsymmetricStacks`.
#[test]
fn start_stack_follows_dealt_order_not_seat_ids() {
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let dealt = vec![Seat(1), Seat(3), Seat(5)]; // SB 1, BB 3, BTN 5
    let stacks = vec![100 * unit, 150 * unit, 100 * unit]; // the BB seat is the deep one
    let state = core_model::begin_hand(
        &cfg,
        BeginHand { hand_id: 1, button: Seat(5), hero: Seat(1), dealt, stacks_start: stacks, hero_cards: None },
    )
    .expect("the model admits this table");
    assert_eq!(core_preflop::start_stack(&state, Seat(3)), 150 * unit);
    assert_eq!(core_preflop::start_stack(&state, Seat(5)), 100 * unit);
    let answer = synthetic_store().query(&cfg, &state, 0);
    assert_eq!(answer.actor, Some(Seat(5)), "the button acts first three-handed: {answer:?}");
    assert!(
        answer.reasons.contains(&ApproxReason::AsymmetricStacks { stacks_bb: vec![100.0, 150.0, 100.0], prominent: true }),
        "{:?}",
        answer.reasons
    );
    assert!(
        !answer.reasons.iter().any(|r| matches!(r, ApproxReason::DepthBucket { .. })),
        "the button is covered to exactly 100 bb: {:?}",
        answer.reasons
    );
}

#[test]
fn straddle_maps_virtual_roles_and_keeps_physical_posts() {
    let unit = 2000u32; // the straddle is the source unit
    let cfg = config(500, 1000, Some(unit), exact_rake(unit));
    let state = table(&cfg, 6, &vec![100 * unit; 6]);
    // The physical posts are untouched by the virtual-role mapping.
    assert_eq!(state.derived.committed_this_street[0], 500, "the SB seat posts the small blind");
    assert_eq!(state.derived.committed_this_street[1], 1000, "the BB seat posts the big blind");
    assert_eq!(state.derived.committed_this_street[2], 2000, "the UTG seat posts the straddle");
    assert_eq!(core_preflop::source_unit(&cfg), unit);
    // HJ (virtual UTG) opens to 2.5 straddles; the CO seat (virtual HJ) is then to act.
    let opened = act(&state, &[Action::Raise { to: 5000 }]);
    assert_eq!(opened.actions[0].seat, Seat(3), "the HJ seat acts first behind a straddle");
    let answer = synthetic_store().query(&cfg, &opened, 1);
    assert_eq!(answer.unsupported, None, "{answer:?}");
    assert_eq!(answer.actor, Some(Seat(4)));
    assert_eq!(answer.unit, unit);
    assert_eq!(answer.node.as_ref().map(|n| n.actor), Some(Position::Hj), "the virtual HJ node");
    assert_eq!(
        answer.key,
        node_key(&PreflopNodeKey {
            depth_bb: 100,
            rake_profile: "5% cap 0.5bb".into(),
            straddle: false, // the key's straddle flag is the candidate's, never the live hand's
            history: vec![(Position::Utg, PreflopStep::Raise { to_bb_x1000: 2500 })],
        })
    );
    assert!(answer.reasons.contains(&ApproxReason::StraddleMapped { posts: [0.25, 0.5, 1.0] }), "{:?}", answer.reasons);
}

#[test]
fn ranking_is_deterministic_and_misses_are_never_hidden() {
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let state = table(&cfg, 6, &vec![100 * unit; 6]);

    let empty = PreflopStore::from_sources(vec![]);
    assert_eq!(
        empty.query(&cfg, &state, 0).unsupported,
        Some(UnsupportedReason::MissingPreflopNode { key: "no bundle".into() })
    );

    // Equal rank: the lexicographically smaller bundle id wins, and its miss is final.
    let tie = PreflopStore::from_sources(vec![variant("b", |_| {}), variant("a", |_| {})]);
    let answer = tie.query(&cfg, &state, 0);
    assert_eq!(answer.bundle.as_ref().map(|b| b.bundle_id.as_str()), Some("a"), "{answer:?}");
    assert!(matches!(answer.unsupported, Some(UnsupportedReason::MissingPreflopNode { .. })), "{answer:?}");

    let mixed = PreflopStore::from_sources(vec![variant("a", |_| {}), synthetic_bundle()]);
    let answer = mixed.query(&cfg, &state, 0);
    assert_eq!(answer.bundle.as_ref().map(|b| b.bundle_id.as_str()), Some("a"), "{answer:?}");
    assert!(answer.node.is_none(), "a lower-ranked bundle is never searched to hide a miss: {answer:?}");

    // PokerData sorts ahead of a chart transcription at the same depth and rake, whatever the id.
    let charts_last =
        PreflopStore::from_sources(vec![variant("aaa_chart", |i| i.source = SourceKind::ChartTranscription), synthetic_bundle()]);
    assert_eq!(
        charts_last.query(&cfg, &state, 0).bundle.map(|b| b.bundle_id),
        Some("synthetic_v2_100bb".to_string())
    );

    // Nearest depth wins, ties deeper, and the label says so.
    let shallow = PreflopStore::from_sources(vec![variant("a", |i| i.depths = vec![50, 150])]);
    let answer = shallow.query(&cfg, &state, 0);
    assert!(
        answer.reasons.contains(&ApproxReason::DepthBucket {
            seat: Seat(2),
            actual_bb: 100.0,
            used_bb: 150,
            prominent: true
        }),
        "{:?}",
        answer.reasons
    );

    let no_depth = PreflopStore::from_sources(vec![variant("a", |i| i.depths = vec![])]);
    assert_eq!(
        no_depth.query(&cfg, &state, 0).unsupported,
        Some(UnsupportedReason::MissingPreflopNode { key: "no acquired depth".into() })
    );
}

#[test]
fn a_postflop_prefix_or_a_foreign_config_is_unsupported() {
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let store = synthetic_store();
    let state = table(&cfg, 6, &vec![100 * unit; 6]);

    let other = config(unit / 2, unit, None, Rake::TimeCharge);
    assert_eq!(
        store.query(&other, &state, 0).unsupported,
        Some(UnsupportedReason::UnsupportedHistory { reason: "config does not match the frozen hand config".into() })
    );

    // Preflop closes, a flop is dealt, and a flop action is recorded.
    let closed = act(&state, &[Action::Fold, Action::Fold, Action::Fold, Action::Call, Action::Call, Action::Check]);
    assert_eq!(
        store.query(&cfg, &closed, closed.actions.len()).unsupported,
        Some(UnsupportedReason::UnsupportedHistory { reason: "no actor at prefix".into() })
    );
    let flop = core_model::set_board(&closed, &[Card(0), Card(4), Card(8)]).expect("the flop is dealt");
    let flop = core_model::apply_action(&flop, Action::Check).expect("the SB checks the flop");
    assert_eq!(
        store.query(&cfg, &flop, flop.actions.len()).unsupported,
        Some(UnsupportedReason::UnsupportedHistory { reason: "preflop query over a postflop prefix".into() })
    );
    // The preflop prefixes of that same hand still answer.
    let root = store.query(&cfg, &flop, 0);
    assert!(root.node.is_some(), "truncating to the preflop root answers: {root:?}");
    assert_eq!(root.actor, Some(Seat(2)));
    assert!(store.query(&cfg, &flop, 12).unsupported.is_some(), "a prefix past the history is unsupported");
}

/// R1: a malformed straddle format is a typed rejection on the public `query` path, decided before
/// the prefix is reconstructed -- `prefix_state` -> `core_model::derive` would panic on exactly
/// these states, because model admission refuses them.
#[test]
fn query_rejects_malformed_straddle_formats_without_unwinding() {
    let store = synthetic_store();
    for (expected, state) in [
        ("short straddle post", straddled_state(1, 2, 3, 6, 400)),
        ("straddle requires six dealt seats", straddled_state(1, 2, 4, 5, 400)),
        ("straddler's starting stack does not cover the straddle", straddled_state(1, 2, 4, 6, 3)),
    ] {
        let cfg = state.config.clone();
        let answer = store.query(&cfg, &state, 0);
        match answer.unsupported {
            Some(UnsupportedReason::FormatUnsupported { ref detail }) => {
                assert!(detail.contains(expected), "expected {expected:?} in {detail:?}")
            }
            ref other => panic!("{expected}: expected FormatUnsupported, got {other:?}"),
        }
        assert!(answer.node.is_none() && answer.bundle.is_none(), "{answer:?}");
    }
    // A stack vector that does not match the dealt seats is refused for the same reason: it is the
    // shape `start_stack` and the model replay both require.
    let cfg = config(1, 2, Some(4), Rake::TimeCharge);
    let mismatched = raw_state(cfg.clone(), dealt_seats(6), vec![400; 5], Seat(5));
    let answer = store.query(&cfg, &mismatched, 0);
    match answer.unsupported {
        Some(UnsupportedReason::FormatUnsupported { ref detail }) => {
            assert!(detail.contains("starting stacks"), "{detail:?}")
        }
        ref other => panic!("expected FormatUnsupported, got {other:?}"),
    }
    // Control: a legal straddle answers normally on the same path.
    let unit = 2000u32;
    let legal = config(500, 1000, Some(unit), exact_rake(unit));
    let answer = store.query(&legal, &table(&legal, 6, &vec![100 * unit; 6]), 0);
    assert_eq!(answer.unsupported, None, "{answer:?}");
}

/// R2: a zero acquired depth is a malformed manifest, rejected at admission so the store's
/// quarantine and banner path applies -- never silently skipped during ranking.
#[test]
fn manifest_depths_must_be_positive_at_admission() {
    for (case, depths) in [("zero_only", vec![0u16]), ("zero_and_valid", vec![0, 100])] {
        let dir = temp_bundle(case, &depths);
        let err = load_bundle(&dir.join("synthetic/manifest.json"), &dir.join("synthetic/nodes.json"))
            .err()
            .unwrap_or_else(|| panic!("{case}: a zero acquired depth must be rejected at admission"));
        assert!(err.to_string().contains("depths"), "{case}: {err}");
        let (store, banners) = PreflopStore::open(&dir);
        assert_eq!(store.bundles().len(), 0, "{case}: the malformed bundle stays out of the store");
        assert_eq!(banners.len(), 1, "{case}: {banners:?}");
        assert!(banners[0].contains("depths"), "{case}: {banners:?}");
        assert!(dir.join("synthetic.bad").exists(), "{case}: the bundle is quarantined");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // Control: the committed depths list still admits.
    let dir = temp_bundle("valid", &[100]);
    assert!(load_bundle(&dir.join("synthetic/manifest.json"), &dir.join("synthetic/nodes.json")).is_ok());
    let (store, banners) = PreflopStore::open(&dir);
    assert_eq!((store.bundles().len(), banners.len()), (1, 0), "{banners:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// R2's invariant for an in-memory `BundleInfo` no admission path produced: a zero declared depth
/// is refused, never silently skipped.
#[test]
#[should_panic(expected = "bucket: acquired depth 0 is 0")]
fn bucket_refuses_a_zero_declared_depth() {
    let _ = core_preflop::bucket(100.0, &[0, 100]);
}

/// R3: a historical wager is resolved against the selected source's own menu at the corresponding
/// prefix node, comparing exact chip amounts. Thousandth rounding is never what makes a match.
#[test]
fn historical_sizes_resolve_against_the_source_menu() {
    // The half-chip rule itself, at the boundary the review's arithmetic probe pinned down.
    assert!(core_preflop::size_matches(8, 2500, 3), "8 chips at a 3-chip unit is exactly half a chip from 2.5");
    assert!(core_preflop::size_matches(24, 2450, 10), "exactly half a chip apart is inside the tolerance");
    assert!(!core_preflop::size_matches(7501, 2500, 3000), "a full chip above the source size is off-menu");

    let store = synthetic_store();
    let source_open = |history| {
        node_key(&PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history })
    };

    // Small unit: an 8-chip open at a 3-chip big blind is within half a chip of the source's
    // 2.5 bb open, so it resolves onto 2500. Thousandth rounding would have emitted 2667.
    let cfg = config(1, 3, None, exact_rake(3));
    let opened = act(&table(&cfg, 6, &vec![300; 6]), &[Action::Raise { to: 8 }]);
    let answer = store.query(&cfg, &opened, 1);
    assert_eq!(answer.unsupported, None, "{answer:?}");
    assert_eq!(answer.node.as_ref().map(|n| n.actor), Some(Position::Hj), "{answer:?}");
    assert_eq!(answer.key, source_open(vec![(Position::Utg, PreflopStep::Raise { to_bb_x1000: 2500 })]));

    // Large unit: 7501 chips at a 3000-chip big blind is a full chip above the source's
    // 7500-chip size, so it stays off-menu for Tasks 10/13 and never selects the present 2500
    // node -- which is exactly the key thousandth rounding would have produced.
    let cfg = config(1500, 3000, None, exact_rake(3000));
    let opened = act(&table(&cfg, 6, &vec![300_000; 6]), &[Action::Raise { to: 7501 }]);
    let answer = store.query(&cfg, &opened, 1);
    assert!(answer.node.is_none(), "{answer:?}");
    assert!(matches!(answer.unsupported, Some(UnsupportedReason::MissingPreflopNode { .. })), "{answer:?}");
    assert!(answer.notes.iter().any(|n| n.contains("off-menu")), "{:?}", answer.notes);
    assert_ne!(
        answer.key,
        source_open(vec![(Position::Utg, PreflopStep::Raise { to_bb_x1000: 2500 })]),
        "an off-menu size is never rounded into the source's key"
    );

    // A wager deeper in the history resolves against *that* node's menu, not the root's: at a
    // 3-chip unit a 26-chip 3bet is within half a chip of the source's 8.75 bb size.
    let cfg = config(1, 3, None, exact_rake(3));
    let three_bet = act(&table(&cfg, 6, &vec![300; 6]), &[Action::Raise { to: 8 }, Action::Raise { to: 26 }]);
    let answer = store.query(&cfg, &three_bet, 2);
    assert_eq!(
        answer.key,
        source_open(vec![
            (Position::Utg, PreflopStep::Raise { to_bb_x1000: 2500 }),
            (Position::Hj, PreflopStep::Raise { to_bb_x1000: 8750 }),
        ]),
        "{answer:?}"
    );
    // A 3bet that node does not offer stays off-menu even though the open before it resolved.
    let off = act(&table(&cfg, 6, &vec![300; 6]), &[Action::Raise { to: 8 }, Action::Raise { to: 40 }]);
    let answer = store.query(&cfg, &off, 2);
    assert!(answer.key.contains("off-menu"), "{}", answer.key);
    assert!(answer.key.contains("2500"), "the resolved open is still visible: {}", answer.key);
    assert!(matches!(answer.unsupported, Some(UnsupportedReason::MissingPreflopNode { .. })), "{answer:?}");
    assert!(answer.node.is_none(), "{answer:?}");

    // The inclusive boundary end to end: at a one-chip unit a 3-chip open is exactly half a chip
    // from 2.5 bb and resolves; 4 chips is a chip and a half away and does not.
    let cfg = config(1, 1, None, exact_rake(1));
    let on = act(&table(&cfg, 6, &vec![100; 6]), &[Action::Raise { to: 3 }]);
    let answer = store.query(&cfg, &on, 1);
    assert_eq!(answer.node.as_ref().map(|n| n.actor), Some(Position::Hj), "{answer:?}");
    assert_eq!(answer.key, source_open(vec![(Position::Utg, PreflopStep::Raise { to_bb_x1000: 2500 })]));
    let off = act(&table(&cfg, 6, &vec![100; 6]), &[Action::Raise { to: 4 }]);
    let answer = store.query(&cfg, &off, 1);
    assert!(answer.node.is_none(), "{answer:?}");
    assert!(matches!(answer.unsupported, Some(UnsupportedReason::MissingPreflopNode { .. })), "{answer:?}");
}

/// R4: the public hand domain is wider than the source-key domain, so a legal raise whose size has
/// no `u32` thousandths representation is a typed unsupported answer, never a panic or a clamp.
#[test]
fn an_unrepresentable_source_size_is_typed_not_a_panic() {
    let cfg = config(1, 2, None, exact_rake(2));
    // Six 20,000,000-chip stacks total 120,000,000 -- well inside the aggregate chip bound -- and
    // the 10,000,000-chip raise is not all-in.
    let opened = act(&table(&cfg, 6, &vec![20_000_000; 6]), &[Action::Raise { to: 10_000_000 }]);
    let answer = synthetic_store().query(&cfg, &opened, 1);
    match answer.unsupported {
        Some(UnsupportedReason::UnsupportedHistory { ref reason }) => {
            assert!(reason.contains("representable"), "{reason:?}")
        }
        ref other => panic!("expected UnsupportedHistory, got {other:?}"),
    }
    assert!(answer.node.is_none(), "{answer:?}");
    assert_eq!(answer.unit, 2);
    assert!(!answer.reasons.is_empty(), "the mapping reasons accumulate before the history is resolved");
}

/// R5: the brief's per-invocation mapping cache. It lives in the caller's invocation object, never
/// in the shared store, and Task 13's replay driver will own one per run.
#[test]
fn an_invocation_memoizes_mappings_and_resets_between_runs() {
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let store = synthetic_store();
    let root = table(&cfg, 6, &vec![100 * unit; 6]);
    let opened = act(&root, &[Action::Raise { to: 2500 }]);

    let mut run = core_preflop::PreflopInvocation::new();
    assert!(run.is_empty());
    let first = run.answer(&store, &cfg, &opened, 1);
    let second = run.answer(&store, &cfg, &opened, 1);
    assert_eq!(first, second, "the second call is the memoized mapping");
    assert_eq!(first, store.query(&cfg, &opened, 1), "the memo returns exactly what the store would");
    assert_eq!((run.lookups(), run.hits(), run.len()), (2, 1, 1));

    // A different prefix of the same hand, and a different branch-translated history, are separate
    // entries rather than a reused mapping.
    let _ = run.answer(&store, &cfg, &opened, 0);
    let other_branch = act(&root, &[Action::Call]);
    let limp = run.answer(&store, &cfg, &other_branch, 1);
    assert_eq!((run.lookups(), run.hits(), run.len()), (4, 1, 3));
    assert_ne!(limp.key, first.key, "a different translated history is a different mapping");

    // Reset between runs: nothing carries over, and the next call is a fresh miss.
    run.reset();
    assert_eq!((run.lookups(), run.hits(), run.len()), (0, 0, 0));
    assert!(run.is_empty());
    let _ = run.answer(&store, &cfg, &opened, 1);
    assert_eq!((run.lookups(), run.hits(), run.len()), (1, 0, 1));
}

/// `observed_history` stays the menu-unaware view of a prefix (Tasks 10/13 read branch histories
/// through it); `query` uses the menu-aware resolution instead.
#[test]
fn observed_history_is_the_menu_unaware_view() {
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let opened = act(&table(&cfg, 6, &vec![100 * unit; 6]), &[Action::Raise { to: 2500 }, Action::Call]);
    let prefix = core_preflop::prefix_state(&opened, 2);
    let roles = core_preflop::physical_positions(&prefix);
    assert_eq!(
        core_preflop::observed_history(&prefix, &roles, false, unit),
        vec![(Position::Utg, PreflopStep::Raise { to_bb_x1000: 2500 }), (Position::Hj, PreflopStep::Call)]
    );
    // The same actions under the virtual roles a straddle imposes.
    assert_eq!(core_preflop::observed_history(&prefix, &roles, true, unit)[0].0, Position::Bb);
}

/// Why `query` screens for a postflop action *before* it reconstructs the prefix: `derive` cannot
/// replay a postflop action with the board cleared, so `prefix_state` refuses one outright instead
/// of panicking inside `core_model`.
#[test]
#[should_panic(expected = "prefix_state: action 6 is on Flop, not preflop")]
fn prefix_state_refuses_a_postflop_prefix() {
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let state = table(&cfg, 6, &vec![100 * unit; 6]);
    let closed = act(&state, &[Action::Fold, Action::Fold, Action::Fold, Action::Call, Action::Call, Action::Check]);
    let flop = core_model::set_board(&closed, &[Card(0), Card(4), Card(8)]).expect("the flop is dealt");
    let flop = core_model::apply_action(&flop, Action::Check).expect("the SB checks the flop");
    let _ = core_preflop::prefix_state(&flop, flop.actions.len());
}
