//! Spec section 13.1 T4 `cache_key_structural_identity` (plan 4 Task 8): the complete structural-identity scale
//! contract of section 10.4, frozen over the real cache lookup of Task 7.
//!
//! Contract (spec section 10.4, copied verbatim):
//!
//! **Scale check** (contract, section 13.1 T4): pot/stack/cap 100/500/5000 mchips and 200/1000/10000 mchips with
//! identical ranges, proportional quantum and identical fractional tree hit the same entry with identical frequencies
//! and doubled chip EV, `Exact` (identical SPR rationals, identical realized fractions: every pot and stack doubles
//! exactly, so `max(dev) = 0` at every node), at the root, at IP's node after Bet(50) versus Bet(100), and at OOP's
//! node after Check, Bet(50) versus Check, Bet(100) (both actors at non-root nodes; a call closes the street, so the
//! next decision lives in the next street's own root and is never requested from this entry, section 5); 100/500
//! versus 103/515 with a one-chip quantum has SPR `5 : 1` in both and rounds the root half-pot bet to 50 versus 52
//! chips (52/103 = 0.5049, a **root-action** deviation of 0.49%); the reported `MenuRounded{max_delta_pct}` is the
//! maximum over the complete materialized list and is larger: after bet/call the turn pots are 200 and 207, the
//! half-pot bets 100 and 104 (`104/103 - 100/100 = 0.0097`, 0.97%), and deeper nodes deviate further; T4 freezes the
//! value computed from the two materialized lists (asserting `0.97 <= max_delta_pct < 5`), never the root number, and
//! the candidate is `Approximate{MenuRounded}`, never `Exact`; 100/500 versus 100/508 (SPR 5.08, `delta = 1.6%`) is
//! `SprBucketed` (and `MenuRounded` only if some realized size actually differs); 100/500 versus 100/511
//! (`delta = 2.2%`) misses; the `MenuRounded{2.0}` case is a specified pair of trees: test template
//! `menu_round_test_v1` (single 0.33 bet, all-in-only raises, cap 1, add-all-in 1.5, force-all-in 0.15, flop root) at
//! entry 100/500 versus query 20/100 (SPR `5 : 1` in both): the 0.33 bet is 33 chips (0.33) versus 7 chips (0.35) at
//! every node with an unchanged pot (`dev = 0.02`), 55 versus 11 (0.55 in both) after bet/call, 91 versus 18 (0.91
//! versus 0.90) after bet/call/bet/call, the all-in raise-to amounts deviate by at most 0.02 (467 versus 93 after
//! bet/call), and the river all-in is added in both trees after bet/call/bet/call (`412 <= 414`, `82 <= 84`) and in
//! neither elsewhere, so the topologies agree and `max(dev) = 0.02`; a single-node comparison is never used, so a query
//! whose root bet is exactly representable still carries `MenuRounded` when a deeper node rounds differently; a
//! different canonical board always misses.
//!
//! Every lookup here goes through the production path: `CacheRig` (tests/support) materializes the real template with
//! the production tree builder, stores one validated `CacheEntry` of synthetic, node-distinct matrices through the
//! real cache writer, and builds each query with `engine::cache_bridge::make_cache_query`; no worker solve and no
//! timing noise enters T4.
//!
//! **Where the realized trees contradict the contract text (plan 4 Task 8 report, concern 1).** The contract's `Exact`
//! for 100/500 versus 200/1000 rests on its own premise "identical fractional tree ... every pot and stack doubles
//! exactly". Under `flop_fast_v1` (bets 0.5, raises 2.5x, cap 3) that premise is false: the flop 3-bet after Bet(50),
//! Raise(125) is `round(2.5 * 125 = 312.5) = 313` chips at P = 100 (section 4.6: `f64::round`, half away from zero)
//! but `2.5 * 250 = 625` at P = 200, and every later all-in inherits the half chip (187 versus 375). The normative
//! label rule of section 10.4 ("`Exact` iff ... `max(dev) == 0`", every action of every node) therefore makes the
//! doubled query `Approximate{MenuRounded{0.5}}` (`|625/200 - 313/100| = 1/200`), and the production lookup says so.
//! `scale_check_on_flop_fast_v1_serves_the_same_entry_with_doubled_ev` asserts that rule-derived value (computed
//! independently below) together with every other clause of the contract (same entry, same node, identical
//! frequencies, doubled EV, no `SprBucketed`); `scale_check_is_exact_where_the_fractional_tree_doubles_exactly` asserts
//! the contract's `Exact` on the one T4 template whose fractional tree does double exactly (`check_jam_test_v1`).

pub mod support;

use cache::key::{Model, Rational};
use cache::lookup::{compare, CacheHit, CacheQuery, Lookup, MissReason};
use core_iso::SuitPerm;
use proto::{Action, ApproxReason, Card, Coverage, MaterializedNode, Seat, Street};
use support::{cards, CacheRig, Scenario, BUDGET, EXPLOITABILITY_OVER_P, IP, OOP};

// --- helpers ------------------------------------------------------------------------------------------------------

/// The node a hit served for its query.
fn served(hit: &CacheHit) -> &proto::worker::NodeStrategy {
    &hit.solution.nodes[hit.solution.requested as usize]
}

/// The largest menu deviation of section 10.4 between two positionally aligned materialized lists, computed here
/// independently of `cache::lookup::compare`: exact integers `|to_q * P_e - to_e * P_q|` over the denominator
/// `P_e * P_q`, and the first node/action (in list order) attaining it.
#[derive(Debug, PartialEq)]
struct Deviation {
    num: u128,
    den: u128,
    path: Vec<u8>,
    index: usize,
    entry_action: Action,
    query_action: Action,
}

fn max_deviation(entry: &[MaterializedNode], pe: u32, query: &[MaterializedNode], pq: u32) -> Deviation {
    assert_eq!(entry.len(), query.len(), "the lists have one node set");
    let mut best: Option<Deviation> = None;
    for (a, b) in entry.iter().zip(query) {
        assert_eq!((&a.path, a.actions.len()), (&b.path, b.actions.len()), "the lists are aligned");
        for (i, (x, y)) in a.actions.iter().zip(&b.actions).enumerate() {
            if let (Some(tx), Some(ty)) = (cache::lookup::action_to(x), cache::lookup::action_to(y)) {
                let num = (ty as u128 * pe as u128).abs_diff(tx as u128 * pq as u128);
                if best.as_ref().is_none_or(|d| num > d.num) {
                    best = Some(Deviation { num, den: pe as u128 * pq as u128, path: a.path.clone(), index: i, entry_action: *x, query_action: *y });
                }
            }
        }
    }
    best.expect("the trees have wagers")
}

fn node<'a>(list: &'a [MaterializedNode], path: &[u8]) -> &'a MaterializedNode {
    list.iter().find(|n| n.path == path).unwrap_or_else(|| panic!("no node at {path:?}"))
}

/// Whether two materialized lists have equal section 2 topology (paths, actors, streets, action kinds, terminal
/// classification): the direct check a boundary pair must fail, asserted independently of `compare`.
fn same_topology(a: &[MaterializedNode], b: &[MaterializedNode]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.path == y.path
                && x.actor == y.actor
                && x.street == y.street
                && x.actions.len() == y.actions.len()
                && x.actions.iter().zip(&y.actions).all(|(p, q)| std::mem::discriminant(p) == std::mem::discriminant(q))
                && x.terminal_pots.iter().zip(&y.terminal_pots).all(|(p, q)| p.is_some() == q.is_some())
        })
}

fn assert_no_match(rig: &CacheRig, q: &CacheQuery, what: &str) {
    assert_eq!(rig.lookup(q), Lookup::Miss { reason: MissReason::NoMatch }, "{what}");
}

/// The synthetic EV of the brief (`EV(a) = 10 * n + a`, fold 0) at P = `p`, for node index `n` of the entry.
fn expected_ev(n: usize, actions: &[Action]) -> Vec<f32> {
    actions.iter().enumerate().map(|(i, a)| if *a == Action::Fold { 0.0 } else { (10 * n + i) as f32 }).collect()
}

// --- the brief's scale / actor test (spec 13.1 T4) ---------------------------------------------------------------

/// Brief step 2, with Task 7's `Option<Coverage>`, plus brief step 5's golden. The doubled-scale label is the one the
/// section 10.4 rule derives for `flop_fast_v1` (module doc); every other clause is asserted as written.
#[test]
fn cache_key_structural_identity() {
    let rig = CacheRig::new("flop_fast_v1", 100, 500, 5000);
    for (small, large, actor) in [
        (vec![], vec![], "oop"),
        (vec![Action::Bet { to: 50 }], vec![Action::Bet { to: 100 }], "ip"),
        (vec![Action::Check, Action::Bet { to: 50 }], vec![Action::Check, Action::Bet { to: 100 }], "oop"),
        (vec![Action::Check], vec![Action::Check], "ip"),
    ] {
        let a = rig.hit(&rig.query(100, 500, 5000, &small, actor));
        let b = rig.hit(&rig.query(200, 1000, 10000, &large, actor));
        assert!(matches!(a.coverage, Some(Coverage::Exact)), "{small:?}: the entry's own scale is Exact, got {:?}", a.coverage);
        assert_eq!(b.coverage, Some(Coverage::Approximate { reasons: vec![ApproxReason::MenuRounded { max_delta_pct: 0.5 }] }), "{large:?}");
        let na = served(&a);
        let nb = served(&b);
        assert_eq!(na.actor, actor);
        assert_eq!(na.probs, nb.probs);
        for (x, y) in na.ev_chips.iter().flatten().zip(nb.ev_chips.iter().flatten()) {
            assert!((2.0 * x - y).abs() < 1e-5);
        }
    }
    let rounded = rig.hit(&rig.query(103, 515, 5150, &[], "oop"));
    let value = match &rounded.coverage {
        Some(Coverage::Approximate { reasons }) => reasons
            .iter()
            .find_map(|r| if let ApproxReason::MenuRounded { max_delta_pct } = r { Some(*max_delta_pct) } else { None })
            .unwrap(),
        other => panic!("rounded menu cannot be Exact, got {other:?}"),
    };
    assert!((0.97..5.0).contains(&value), "max_delta_pct {value}");
    assert!(100.0 * (52.0_f32 / 103.0 - 50.0 / 100.0) < value);
    let spr_rig = CacheRig::new("check_only_test_v1", 100, 500, 5000);
    let bucketed = spr_rig.hit(&spr_rig.query(100, 508, 5000, &[], "oop"));
    assert!(matches!(bucketed.coverage, Some(Coverage::Approximate { .. })));
    assert!(spr_rig.miss(&spr_rig.query(100, 511, 5000, &[], "oop")));
    let menu = CacheRig::new("menu_round_test_v1", 100, 500, 5000);
    let hit = menu.hit(&menu.query(20, 100, 1000, &[], "oop"));
    let text = serde_json::to_string(&hit.coverage).unwrap();
    assert!(text.contains("MenuRounded"));
    let a = menu.query(20, 100, 1000, &[], "oop");
    let d = compare(&menu.entry, &a.tree, 20, 100, 1000).unwrap();
    assert!((d.max_dev * 100.0 - 2.0).abs() < 1e-9);

    // Brief step 5: the complete materialized lists, the computed maximum and where it is attained, frozen once.
    let query = rig.query(103, 515, 5150, &[], "oop");
    let dev = max_deviation(&rig.entry.tree.materialized, 100, &query.tree.materialized, 103);
    let exact = compare(&rig.entry, &query.tree, 103, 515, 5150).expect("103/515 is a candidate");
    assert_eq!((exact.menu_num, exact.menu_den), (dev.num, dev.den), "compare's maximum is the independent one");
    assert_eq!(value, (100.0 * dev.num as f64 / dev.den as f64) as f32, "the label discloses that maximum");
    let frozen = serde_json::json!({
        "entry": rig.entry.tree.materialized,
        "query": query.tree.materialized,
        "max_delta_pct": value,
        "argmax": { "path": dev.path, "action_index": dev.index, "entry_action": dev.entry_action, "query_action": dev.query_action,
            "dev_num": dev.num as u64, "dev_den": dev.den as u64 },
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/cache_scale.json");
    if std::env::var_os("POKERAI_RECORD_GOLDENS").is_some() {
        std::fs::write(&path, serde_json::to_vec_pretty(&frozen).unwrap()).unwrap();
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("the committed golden {} is missing ({e}); record it once with POKERAI_RECORD_GOLDENS=1 and inspect it", path.display()));
    let expected: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(frozen, expected);
}

// --- scale, actor and node selection ------------------------------------------------------------------------------

/// The contract's four requested nodes under `flop_fast_v1` at 100/500 versus 200/1000: the same entry serves the same
/// node (its synthetic EV names it) for the right actor, with identical frequencies and exactly doubled chip EV; the
/// SPR rationals are identical (no `SprBucketed`), the requested nodes' own realized fractions are identical, and the
/// one disclosed reason is the rule-derived `MenuRounded{0.5}` of the flop 3-bet 313 versus 625 (module doc): a
/// single-node comparison is never used.
#[test]
fn scale_check_on_flop_fast_v1_serves_the_same_entry_with_doubled_ev() {
    let rig = CacheRig::new("flop_fast_v1", 100, 500, 5000);
    let doubled = rig.query(200, 1000, 10000, &[], "oop");
    assert_eq!(doubled.source.spr, rig.entry.source.spr, "SPR 5 : 1 at both scales");
    let dev = max_deviation(&rig.entry.tree.materialized, 100, &doubled.tree.materialized, 200);
    assert_eq!((dev.num, dev.den), (100, 20_000), "max(dev) = 1/200");
    // first attained (list order) at the river 3-bet after four checks; the flop 3-bets round the same way
    assert_eq!((dev.path.as_slice(), dev.entry_action, dev.query_action), (&[0u8, 0, 0, 0, 0, 1, 2][..], Action::Raise { to: 313 }, Action::Raise { to: 625 }));
    for flop_three_bet in [&[0u8, 1, 2][..], &[1, 2][..]] {
        assert_eq!((node(&rig.entry.tree.materialized, flop_three_bet).actions[2], node(&doubled.tree.materialized, flop_three_bet).actions[2]), (Action::Raise { to: 313 }, Action::Raise { to: 625 }));
    }
    assert_eq!(node(&rig.entry.tree.materialized, &[0, 1]).actions, vec![Action::Fold, Action::Call, Action::Raise { to: 125 }], "IP's 50 raised to 2.5 x 50");
    assert!(same_topology(&rig.entry.tree.materialized, &doubled.tree.materialized));

    for (small, large, actor) in [
        (vec![], vec![], "oop"),
        (vec![Action::Bet { to: 50 }], vec![Action::Bet { to: 100 }], "ip"),
        (vec![Action::Check, Action::Bet { to: 50 }], vec![Action::Check, Action::Bet { to: 100 }], "oop"),
        (vec![Action::Check], vec![Action::Check], "ip"),
    ] {
        let qa = rig.query(100, 500, 5000, &small, actor);
        let qb = rig.query(200, 1000, 10000, &large, actor);
        assert_eq!(qa.key.digest(), qb.key.digest(), "one key at both scales");
        assert_eq!(qa.requested, qb.requested, "one ordinal node");
        let n = rig.entry.nodes.iter().position(|m| m.path == qa.requested).expect("a covered node");
        let (a, b) = (rig.hit(&qa), rig.hit(&qb));
        assert!(matches!(rig.lookup(&qa), Lookup::Exact { .. }));
        assert!(matches!(rig.lookup(&qb), Lookup::Approximate { ref reasons, .. } if *reasons == vec![ApproxReason::MenuRounded { max_delta_pct: 0.5 }]));
        let (na, nb) = (served(&a), served(&b));
        assert_eq!((na.actor.as_str(), nb.actor.as_str()), (actor, actor));
        // the requested node's own realized fractions are identical at both scales
        assert_eq!(na.actions.iter().map(|x| cache::lookup::action_to(x).map(|t| 2 * t)).collect::<Vec<_>>(), nb.actions.iter().map(cache::lookup::action_to).collect::<Vec<_>>());
        assert_eq!(na.probs, nb.probs, "identical frequencies");
        assert_eq!(na.available, nb.available);
        for c in 0..1326 {
            for (x, y) in na.ev_chips[c].iter().zip(&nb.ev_chips[c]) {
                assert_eq!(2.0 * x, *y, "doubled chip EV, combo {c}");
            }
            if na.available[c] {
                for (x, want) in na.ev_chips[c].iter().zip(expected_ev(n, &na.actions)) {
                    assert!((x - want).abs() < 1e-4, "node {n} ({:?}) served for {actor}: EV {x} is not the node's own {want}", qa.requested);
                }
            }
        }
    }
}

/// The contract's `Exact` across scales, on the T4 template whose fractional tree doubles exactly at 100/500 versus
/// 200/1000 (`check_jam_test_v1`: every wager is an all-in to the remaining stack): the same entry is `Exact` at both
/// scales, at the root and at IP's node after the jam, with identical frequencies and doubled EV.
#[test]
fn scale_check_is_exact_where_the_fractional_tree_doubles_exactly() {
    let rig = CacheRig::new("check_jam_test_v1", 100, 500, 5000);
    for (small, large, actor) in [(vec![], vec![], "oop"), (vec![Action::AllIn { to: 500 }], vec![Action::AllIn { to: 1000 }], "ip")] {
        let qa = rig.query(100, 500, 5000, &small, actor);
        let qb = rig.query(200, 1000, 10000, &large, actor);
        let d = max_deviation(&rig.entry.tree.materialized, 100, &qb.tree.materialized, 200);
        assert_eq!(d.num, 0, "every realized fraction is identical");
        let (a, b) = match (rig.lookup(&qa), rig.lookup(&qb)) {
            (Lookup::Exact { hit: a }, Lookup::Exact { hit: b }) => (a, b),
            other => panic!("{actor}: both scales are Exact, got {other:?}"),
        };
        assert_eq!((a.coverage.clone(), b.coverage.clone()), (Some(Coverage::Exact), Some(Coverage::Exact)));
        let (na, nb) = (served(&a), served(&b));
        assert_eq!(na.actor, actor);
        assert_eq!(na.probs, nb.probs);
        for (x, y) in na.ev_chips.iter().flatten().zip(nb.ev_chips.iter().flatten()) {
            assert_eq!(2.0 * x, *y);
        }
    }
}

// --- rake cap, SPR and menu predicates ----------------------------------------------------------------------------

/// Section 10.4 (1b): check/jam at P = 100, stacks 500 versus 504, 5% rake, cap 55.2 chips. Topology is identical and
/// the SPR delta is 0.8%, but the called-jam pots 1,100 and 1,108 rake 55.0 (below the cap) and 55.4 (capped): the
/// terminal rake-cap predicate alone rejects the candidate.
#[test]
fn the_rake_cap_predicate_rejects_the_check_jam_pair_500_504() {
    let rig = CacheRig::new("check_jam_test_v1", 100, 500, 55_200);
    let q = rig.query(100, 504, 55_200, &[], "oop");
    assert_eq!(q.key.at_bucket(0), rig.entry.key.at_bucket(0), "the same key but for the bucket");
    assert!(same_topology(&rig.entry.tree.materialized, &q.tree.materialized), "identical topology");
    assert_eq!((node(&rig.entry.tree.materialized, &[1]).terminal_pots[1], node(&q.tree.materialized, &[1]).terminal_pots[1]), (Some(1100), Some(1108)));
    assert!(100 * 4 <= 2 * 500, "SPR delta 4/500 = 0.8% passes");
    assert!(compare(&rig.entry, &q.tree, 100, 504, 55_200).is_none(), "the cap predicate rejects");
    // control: at a query cap both called-jam pots stay below, the same pair is a candidate with dev 4/100
    let control = compare(&rig.entry, &q.tree, 100, 504, 55_600).expect("no cap flip, no rejection");
    assert_eq!((control.menu_num, control.menu_den), (400, 10_000));
    assert_no_match(&rig, &q, "rake-cap flip");
}

/// Section 10.4 (3): under a wagering template the all-in itself is a menu size. AllIn(504) versus AllIn(500) at
/// P = 100 deviates by 0.04 and is served `Approximate{SprBucketed, MenuRounded{4.0}}`; AllIn(508) deviates by 0.08 and
/// fails the separate 0.05 menu filter although its SPR (1.6%) passes: never is menu validation bypassed on an SPR hit.
#[test]
fn the_menu_filter_rejects_allin_500_versus_508_although_the_spr_passes() {
    let rig = CacheRig::new("check_jam_test_v1", 100, 500, 5000);
    let near = rig.query(100, 504, 5000, &[], "oop");
    match rig.lookup(&near) {
        Lookup::Approximate { reasons, .. } => {
            assert_eq!(reasons, vec![ApproxReason::SprBucketed { actual: 5.04, used: 5.0 }, ApproxReason::MenuRounded { max_delta_pct: 4.0 }]);
        }
        other => panic!("504 is served Approximate, got {other:?}"),
    }
    let far = rig.query(100, 508, 5000, &[], "oop");
    assert!(same_topology(&rig.entry.tree.materialized, &far.tree.materialized));
    let d = max_deviation(&rig.entry.tree.materialized, 100, &far.tree.materialized, 100);
    assert_eq!((d.num, d.den, d.entry_action, d.query_action), (800, 10_000, Action::AllIn { to: 500 }, Action::AllIn { to: 508 }), "dev 0.08");
    assert!(100 * 8 <= 2 * 500, "SPR delta 8/500 = 1.6% passes");
    assert!(compare(&rig.entry, &far.tree, 100, 508, 5000).is_none(), "the 0.05 menu filter rejects");
    assert_no_match(&rig, &far, "menu deviation 0.08");
}

/// Section 10.4 (2): with identical realized menus (`check_only_test_v1`), SPR 5.08 is served with `SprBucketed` and
/// nothing else, and SPR 5.11 misses.
#[test]
fn spr_5_08_is_spr_bucketed_alone_and_5_11_misses() {
    let rig = CacheRig::new("check_only_test_v1", 100, 500, 5000);
    let q = rig.query(100, 508, 5000, &[], "oop");
    assert_ne!(q.key.spr_bucket, rig.entry.key.spr_bucket, "a neighbouring bucket");
    let c = compare(&rig.entry, &q.tree, 100, 508, 5000).unwrap();
    assert_eq!((c.delta_num, c.delta_den, c.menu_num), (800, 50_000, 0), "delta 1.6%, no menu deviation");
    match rig.lookup(&q) {
        Lookup::Approximate { reasons, hit } => {
            assert_eq!(reasons, vec![ApproxReason::SprBucketed { actual: 5.08, used: 5.0 }]);
            assert_eq!(hit.coverage, Some(Coverage::Approximate { reasons }));
        }
        other => panic!("5.08 is SprBucketed, got {other:?}"),
    }
    let far = rig.query(100, 511, 5000, &[], "oop");
    assert!(compare(&rig.entry, &far.tree, 100, 511, 5000).is_none(), "delta 2.2%");
    assert_no_match(&rig, &far, "SPR 5.11");
}

/// The specified `MenuRounded{2.0}` pair (`menu_round_test_v1`, 100/500 versus 20/100): the spec's own witnesses are
/// checked on the two real materialized lists, the topologies agree and the maximum deviation is exactly 0.02.
#[test]
fn the_menu_round_pair_carries_menu_rounded_2_0() {
    let rig = CacheRig::new("menu_round_test_v1", 100, 500, 5000);
    let q = rig.query(20, 100, 1000, &[], "oop");
    let (e, m) = (&rig.entry.tree.materialized, &q.tree.materialized);
    assert!(same_topology(e, m));
    assert_eq!((node(e, &[]).actions[1], node(m, &[]).actions[1]), (Action::Bet { to: 33 }, Action::Bet { to: 7 }));
    // after bet/call the turn bet is 55 versus 11; after bet/call/bet/call the river bet is 91 versus 18 and the river
    // all-in is added in both trees (412 <= 414, 82 <= 84)
    assert_eq!((node(e, &[1, 1]).actions[1], node(m, &[1, 1]).actions[1]), (Action::Bet { to: 55 }, Action::Bet { to: 11 }));
    assert_eq!(node(e, &[1, 1, 1, 1]).actions, vec![Action::Check, Action::Bet { to: 91 }, Action::AllIn { to: 412 }]);
    assert_eq!(node(m, &[1, 1, 1, 1]).actions, vec![Action::Check, Action::Bet { to: 18 }, Action::AllIn { to: 82 }]);
    // the all-in raise facing the turn bet after bet/call: 467 versus 93
    assert_eq!((node(e, &[1, 1, 1]).actions[2], node(m, &[1, 1, 1]).actions[2]), (Action::AllIn { to: 467 }, Action::AllIn { to: 93 }));
    let d = max_deviation(e, 100, m, 20);
    assert_eq!((d.num, d.den), (40, 2_000), "max(dev) = 0.02");
    match rig.lookup(&q) {
        Lookup::Approximate { reasons, .. } => assert_eq!(reasons, vec![ApproxReason::MenuRounded { max_delta_pct: 2.0 }]),
        other => panic!("the pair is MenuRounded{{2.0}}, got {other:?}"),
    }
}

// --- key fields: every one is a miss when it differs ------------------------------------------------------------

/// Brief step 4's nine query mutations, each on a fresh valid copy changing only the indicated field; the unmutated
/// query is the control that hits.
#[test]
fn every_key_field_and_the_requested_node_miss_when_mutated() {
    let rig = CacheRig::new("flop_fast_v1", 100, 500, 5000);
    let base = || rig.query(100, 500, 5000, &[], "oop");
    assert!(matches!(rig.lookup(&base()), Lookup::Exact { .. }), "control");
    let mutations: Vec<(&str, Box<dyn Fn(&mut CacheQuery)>)> = vec![
        ("adapter_version", Box::new(|q| q.key.adapter_version += 1)),
        ("rules_version", Box::new(|q| q.key.rules_version += 1)),
        ("schema_version", Box::new(|q| q.key.schema_version += 1)),
        ("solver_commit", Box::new(|q| q.key.solver_commit.push('x'))),
        ("model", Box::new(|q| q.key.model = Model::Locked { fingerprint: [9; 32] })),
        ("rake.collection_rule_version", Box::new(|q| q.key.rake.collection_rule_version += 1)),
        ("range_hash_oop", Box::new(|q| q.key.range_hash_oop[0] ^= 1)),
        ("actor", Box::new(|q| q.actor = "ip".into())),
        ("requested", Box::new(|q| q.requested = vec![255])),
    ];
    for (field, mutate) in mutations {
        let mut q = base();
        mutate(&mut q);
        assert!(rig.miss(&q), "{field}");
        assert_no_match(&rig, &q, field);
    }
}

/// Brief step 4: a genuinely different canonical flop, the root street, the tree signature, the rake rate and the
/// normalized cap, the IP range hash, and one locked fingerprint against another all miss; nothing is substituted.
#[test]
fn board_street_signature_rake_ranges_and_lock_fingerprint_variations_miss() {
    let rig = CacheRig::new("flop_fast_v1", 100, 500, 5000);
    let base = rig.at(100, 500, 5000);
    let control = base.query("oop");
    assert!(matches!(rig.lookup(&control), Lookup::Exact { .. }));

    let other_flop = Scenario { board: cards("Qs 8h 3d"), ..base.clone() }.query("oop");
    assert_ne!(other_flop.key.canonical_board, control.key.canonical_board);
    assert_no_match(&rig, &other_flop, "a different canonical flop");

    let mut turn = base.query("oop");
    turn.key.root_street = Street::Turn;
    assert_no_match(&rig, &turn, "root street");

    let other_template = Scenario { template: "flop_min_v1".into(), ..base.clone() }.query("oop");
    assert_ne!(other_template.key.tree_signature, control.key.tree_signature);
    assert_no_match(&rig, &other_template, "a different tree signature");
    let mut signature = base.query("oop");
    signature.key.tree_signature.push('0');
    assert_no_match(&rig, &signature, "tree_signature field");

    let rate = Scenario { rake: proto::Rake::PotRake { rate: 0.04, cap_mchips: 5000, no_flop_no_drop: true }, ..base.clone() }.query("oop");
    assert_ne!(rate.key.rake.clone().rate(), control.key.rake.clone().rate());
    assert_no_match(&rig, &rate, "rake rate");

    let cap = rig.query(100, 500, 6000, &[], "oop");
    assert_eq!(cap.key.rake.cap_over_p, Rational::new(6000, 100_000).unwrap());
    assert_no_match(&rig, &cap, "normalized rake cap");

    let ip_range = Scenario { ranges: [base.ranges[0].clone(), core_ranges::parse_range("QQ").unwrap()], ..base.clone() }.query("oop");
    assert_eq!(ip_range.key.range_hash_oop, control.key.range_hash_oop);
    assert_ne!(ip_range.key.range_hash_ip, control.key.range_hash_ip);
    assert_no_match(&rig, &ip_range, "a different IP range");
    let mut ip_hash = base.query("oop");
    ip_hash.key.range_hash_ip[0] ^= 1;
    assert_no_match(&rig, &ip_hash, "range_hash_ip field");

    let locked = CacheRig::with_entry(Scenario::new("flop_fast_v1", 100, 500, 5000), |e| {
        e.key.model = Model::Locked { fingerprint: [7; 32] };
        e.locks_applied = 1;
    });
    let mut same = locked.query(100, 500, 5000, &[], "oop");
    same.key.model = Model::Locked { fingerprint: [7; 32] };
    assert!(matches!(locked.lookup(&same), Lookup::Exact { .. }), "the entry's own lock fingerprint hits");
    let mut other = same.clone();
    other.key.model = Model::Locked { fingerprint: [8; 32] };
    assert_no_match(&locked, &other, "another lock fingerprint");
    let baseline = locked.query(100, 500, 5000, &[], "oop");
    assert_eq!(baseline.key.model, Model::Baseline);
    assert_no_match(&locked, &baseline, "the baseline model against a locked entry");
}

// --- request fields: never in the key -----------------------------------------------------------------------------

/// Brief step 4: hero, seat ids, hand id, `bb_chips`, `target_bp` and the requested path vary separately; only the
/// target and the path change what is served, none changes the key digest. The key's field set is frozen to spec 10.4's
/// twelve, so no field exists that could carry a seat, a hand id, `bb_chips`, raw chips, the quantum, hero, the target
/// or a path; hero's cards reach no query at all (`make_cache_query` takes none), and hero's seat is only ever the
/// acting seat.
#[test]
fn hero_seat_hand_bb_target_and_path_never_enter_the_key() {
    let rig = CacheRig::new("flop_fast_v1", 100, 500, 5000);
    let base = rig.at(100, 500, 5000);
    let q = base.query("oop");
    let digest = q.key.digest();
    let reference = rig.hit(&q);

    let fields = serde_json::to_value(&q.key).unwrap().as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    let mut expected = [
        "schema_version", "solver_commit", "adapter_version", "rules_version", "canonical_board", "root_street", "spr_bucket", "tree_signature", "rake",
        "range_hash_oop", "range_hash_ip", "model",
    ]
    .map(String::from)
    .to_vec();
    expected.sort();
    let mut sorted = fields.clone();
    sorted.sort();
    assert_eq!(sorted, expected, "the key carries exactly the spec 10.4 fields");

    // seats: the two players sit elsewhere (hero included), the same key serves the same node
    let reseated = Scenario { oop: Seat(4), ip: Seat(1), ..base.clone() };
    let rq = reseated.query("oop");
    assert_eq!(rq.key.digest(), digest, "seat ids");
    let rh = rig.hit(&rq);
    assert_eq!((served(&rh).probs.clone(), served(&rh).ev_chips.clone(), rh.coverage.clone()), (served(&reference).probs.clone(), served(&reference).ev_chips.clone(), reference.coverage.clone()));

    // hero: hero is OOP at the root or IP after the check; the street root and its key are the same
    let hero_ip = base.clone().with_history(&[Action::Check]).query("ip");
    assert_eq!(hero_ip.key.digest(), digest, "which seat is hero");
    assert_eq!(served(&rig.hit(&hero_ip)).actor, "ip");

    // bb_chips: diagnostic source metadata only
    let bb = Scenario { bb_chips: 5, ..base.clone() }.query("oop");
    assert_eq!((bb.key.digest(), bb.source.bb_chips), (digest, 5), "bb_chips");
    let bh = rig.hit(&bb);
    assert_eq!((served(&bh).probs.clone(), bh.coverage.clone()), (served(&reference).probs.clone(), reference.coverage.clone()));

    // target: same key, different service (raw 40 bp misses a 30 bp target: Provisional, no coverage claimed)
    let tight = Scenario { target_bp: 30, ..base.clone() }.query("oop");
    assert_eq!((tight.key.digest(), tight.target_bp), (digest, 30), "target_bp");
    match rig.lookup(&tight) {
        Lookup::Provisional { hit, reasons } => {
            assert_eq!((hit.coverage, reasons, hit.raw_exploitability_over_p), (None, vec![], EXPLOITABILITY_OVER_P));
        }
        other => panic!("a 30 bp target is Provisional, got {other:?}"),
    }

    // path: same key, a different node of the same entry
    let after_bet = base.clone().with_history(&[Action::Bet { to: 50 }]).query("ip");
    assert_eq!(after_bet.key.digest(), digest, "requested path");
    assert_ne!(after_bet.requested, q.requested);
    let ah = rig.hit(&after_bet);
    assert_eq!(served(&ah).path, vec![Action::Bet { to: 50 }]);
    assert_ne!(served(&ah).ev_chips, served(&reference).ev_chips, "a different node");

    // raw chips and the proportional quantum: 1/100 versus 1/200 is diagnostic, one-chip wagers in both
    let doubled = rig.query(200, 1000, 10000, &[], "oop");
    assert_eq!(doubled.key.digest(), digest, "raw chips and quantum");
    assert_eq!((q.source.quantum_over_p, doubled.source.quantum_over_p), (Rational::new(1, 100).unwrap(), Rational::new(1, 200).unwrap()));

    // hand id: no street-root input carries one (`SolveInput`, `StreetRootSnapshot`, `CacheQuery`), and the frozen
    // field set above leaves the key no place for it; hero's cards likewise (`make_cache_query` has no such input)
    assert_eq!(q.key, bb.key);
}

// --- suit isomorphism -----------------------------------------------------------------------------------------------

/// Every combo row's EV also names its canonical combo (`c / 2048` chips of the entry's pot added to each non-fold
/// action of available combo `c`), so a row served from the wrong combo is observable.
fn combo_distinct(e: &mut cache::entry::CacheEntry) {
    for n in &mut e.nodes {
        let actions = &e.tree.materialized.iter().find(|m| m.path == n.path).unwrap().actions;
        for (c, row) in n.ev_over_P.iter_mut().enumerate() {
            if n.available[c] {
                for (x, a) in row.iter_mut().zip(actions) {
                    if *a != Action::Fold {
                        *x += c as f32 / 2048.0 / 100.0;
                    }
                }
            }
        }
    }
}

/// Brief step 4: the same street root under all 24 suit permutations of its board has one canonical key and hits the
/// one entry `Exact`, and every row -- the named AA and KK combos first -- is recovered exactly in the query's own
/// suits: the permuted query's row at `sigma(c)` is the base query's row at `c`, and both are the entry's canonical
/// row times the query pot.
#[test]
fn all_24_suit_permutations_share_the_key_and_recover_every_named_row() {
    let rig = CacheRig::with_entry(Scenario::new("flop_fast_v1", 100, 500, 5000), combo_distinct);
    let base = rig.at(100, 500, 5000);
    let requests = [(vec![], "oop"), (vec![Action::Bet { to: 50 }], "ip")];
    let named = ["As Ah", "Ad Ac", "Ah Ac", "Ks Kd", "Kd Kc"];
    for (history, actor) in &requests {
        let q0 = base.clone().with_history(history).query(actor);
        let h0 = rig.hit(&q0);
        let canonical = core_iso::inverse(&q0.inverse_perm);
        for images in core_iso::ALL_PERMS {
            let sigma = SuitPerm(images);
            let s = Scenario { board: base.board.iter().map(|c| core_iso::apply(&sigma, *c)).collect(), ranges: [core_iso::apply_range(&sigma, &base.ranges[0]), core_iso::apply_range(&sigma, &base.ranges[1])], ..base.clone() }
                .with_history(history);
            let q = s.query(actor);
            assert_eq!((q.key.digest(), &q.key.canonical_board), (q0.key.digest(), &q0.key.canonical_board), "{sigma:?}");
            let h = match rig.lookup(&q) {
                Lookup::Exact { hit } => hit,
                other => panic!("{sigma:?}: Exact, got {other:?}"),
            };
            for (k, (node, node0)) in h.solution.nodes.iter().zip(&h0.solution.nodes).enumerate() {
                for c in 0..1326u16 {
                    let [x, y] = proto::combo_cards(c);
                    let image = proto::combo_index(core_iso::apply(&sigma, x), core_iso::apply(&sigma, y)) as usize;
                    assert_eq!(node.probs[image], node0.probs[c as usize], "{sigma:?} node {k} combo {c}");
                    assert_eq!(node.ev_chips[image], node0.ev_chips[c as usize], "{sigma:?} node {k} combo {c}");
                    assert_eq!(node.available[image], node0.available[c as usize], "{sigma:?} node {k} combo {c}");
                }
            }
            let entry_node = rig.entry.nodes.iter().find(|m| m.path == q0.requested).unwrap();
            for text in named {
                let [x, y] = <[Card; 2]>::try_from(cards(text)).unwrap();
                let own = proto::combo_index(core_iso::apply(&sigma, x), core_iso::apply(&sigma, y)) as usize;
                let row = &served(&h).ev_chips[own];
                let stored = &entry_node.ev_over_P[proto::combo_index(core_iso::apply(&canonical, x), core_iso::apply(&canonical, y)) as usize];
                assert_eq!(*row, stored.iter().map(|v| v * 100.0).collect::<Vec<_>>(), "{sigma:?} {text}: the entry's canonical row in the query's suits");
                assert_eq!(*row, served(&h0).ev_chips[proto::combo_index(x, y) as usize], "{sigma:?} {text}");
                let supported = (*actor == "oop") == text.starts_with('A') && !text.contains("Kh");
                assert_eq!(served(&h).available[own], supported, "{sigma:?} {text}");
            }
        }
    }
}

// --- topology boundaries --------------------------------------------------------------------------------------------

/// One boundary pair: the rig stored at `entry` (template, P, eff), the query at `query` (P, eff) with the same
/// proportional cap. The two real materialized lists differ in topology at `at` (asserted directly, whatever else
/// would also reject), `compare` rejects, and the lookup is a `NoMatch` miss.
fn boundary(template: &str, entry: (u32, u32), query: (u32, u32), at: &[u8], history: &[Action], actor: &str) -> (Vec<Action>, Vec<Action>) {
    let rig = CacheRig::new(template, entry.0, entry.1, 50 * entry.0);
    let q = rig.query(query.0, query.1, 50 * query.0, history, actor);
    assert_eq!(q.key.at_bucket(0), rig.entry.key.at_bucket(0), "{template}: every non-SPR key field agrees");
    assert!(!same_topology(&rig.entry.tree.materialized, &q.tree.materialized), "{template}: the topology differs");
    let (e, m) = (node(&rig.entry.tree.materialized, at).actions.clone(), node(&q.tree.materialized, at).actions.clone());
    assert!(e.len() != m.len() || e.iter().zip(&m).any(|(x, y)| std::mem::discriminant(x) != std::mem::discriminant(y)), "{template}: the menu at {at:?} changes kind");
    assert!(compare(&rig.entry, &q.tree, query.0, query.1, 50 * query.0).is_none(), "{template}");
    assert_no_match(&rig, &q, template);
    (e, m)
}

/// Brief step 4: real materialized boundary pairs whose SPR passes, each a topology change -- the opening all-in at
/// add 1.5 (150 versus 151), the half-pot bet forced at force 0.15 (80 versus 81), the facing all-in at add 1.0 (stacks
/// 400 versus 401), and a min-size clamp that deduplicates two bet sizes into one (0.33 and 0.75 of a one-chip pot).
#[test]
fn materialized_boundary_pairs_change_topology_and_miss() {
    let spr_passes = |pe: u32, ee: u32, pq: u32, eq: u32| 100 * (eq as u64 * pe as u64).abs_diff(ee as u64 * pq as u64) <= 2 * ee as u64 * pq as u64;

    assert!(spr_passes(100, 150, 100, 151));
    let (e, m) = boundary("flop_min_v1", (100, 150), (100, 151), &[], &[], "oop");
    assert_eq!((e, m), (vec![Action::Check, Action::Bet { to: 75 }, Action::AllIn { to: 150 }], vec![Action::Check, Action::Bet { to: 75 }]));

    assert!(spr_passes(100, 80, 100, 81));
    let (e, m) = boundary("flop_fast_v1", (100, 80), (100, 81), &[], &[], "oop");
    assert_eq!((e, m), (vec![Action::Check, Action::AllIn { to: 80 }], vec![Action::Check, Action::Bet { to: 50 }, Action::AllIn { to: 81 }]));

    assert!(spr_passes(100, 400, 100, 401));
    let (e, m) = boundary("facing_test_v1", (100, 400), (100, 401), &[1], &[Action::Bet { to: 100 }], "ip");
    assert_eq!((e, m), (vec![Action::Fold, Action::Call, Action::Raise { to: 250 }, Action::AllIn { to: 400 }], vec![Action::Fold, Action::Call, Action::Raise { to: 250 }]));

    // SPR 5 : 1 in both: 0.33 of a one-chip pot rounds to 0 and is clamped up to the one-chip minimum, 0.75 rounds to
    // 1, and the two sizes deduplicate into one bet
    assert!(spr_passes(100, 500, 1, 5));
    let (e, m) = boundary("flop_full_v1", (100, 500), (1, 5), &[], &[], "oop");
    assert_eq!((e, m), (vec![Action::Check, Action::Bet { to: 33 }, Action::Bet { to: 75 }], vec![Action::Check, Action::Bet { to: 1 }]));
}

// --- whole-list deviation, never a single node ---------------------------------------------------------------------

/// Brief step 4: P100/stack500 versus P200/stack1000 with an inserted fraction (Bet 73 versus Bet 146, 0.73 at both
/// scales) whose root and whole tree are exact; then a one-chip alteration of one deeper (turn) size in a test
/// materialized query makes the maximum deviation nonzero while the root and the requested node stay exact, so the hit
/// carries `MenuRounded{0.5}`; an alteration beyond 5% of the pot rejects the candidate.
#[test]
fn a_root_exact_query_still_carries_menu_rounded_from_a_deeper_node() {
    let rig = CacheRig::new_at("check_jam_test_v1", 100, 500, 5000, &[Action::Bet { to: 73 }]);
    assert_eq!(rig.entry.tree.inserted, vec![(vec![], "oop".to_string(), Action::Bet { to: 73 })]);
    let q = rig.query(200, 1000, 10000, &[Action::Bet { to: 146 }], "ip");
    assert_eq!(q.key.digest(), rig.entry.key.digest(), "73/100 and 146/200 are one inserted fraction");
    assert_eq!(max_deviation(&rig.entry.tree.materialized, 100, &q.tree.materialized, 200).num, 0);
    assert!(matches!(rig.lookup(&q), Lookup::Exact { .. }));

    // the turn root after Bet(146) / Call: OOP's all-in to the remaining 854 (427 at P = 100), altered by one chip
    let turn = q.tree.materialized.iter().position(|n| n.path == [1, 1]).expect("the turn root after bet/call");
    assert_eq!((q.tree.materialized[turn].street, &q.tree.materialized[turn].actions[..]), (Street::Turn, &[Action::Check, Action::AllIn { to: 854 }][..]));
    assert_eq!(node(&rig.entry.tree.materialized, &[1, 1]).actions, vec![Action::Check, Action::AllIn { to: 427 }]);
    let alter = |by: u32| {
        let mut altered = q.clone();
        for a in altered.tree.materialized[turn].actions.iter_mut() {
            if let Action::AllIn { to } = a {
                *to += by;
            }
        }
        altered
    };
    let one_chip = alter(1);
    assert_eq!(node(&one_chip.tree.materialized, &[]).actions, node(&q.tree.materialized, &[]).actions, "the root is untouched");
    let c = compare(&rig.entry, &one_chip.tree, 200, 1000, 10000).expect("0.5% is within the menu filter");
    assert_eq!((c.menu_num, c.menu_den), (100, 20_000), "max(dev) = 1/200, from the turn node alone");
    match rig.lookup(&one_chip) {
        Lookup::Approximate { reasons, hit } => {
            assert_eq!(reasons, vec![ApproxReason::MenuRounded { max_delta_pct: 0.5 }]);
            assert_eq!(served(&hit).actions, vec![Action::Fold, Action::Call], "the requested node itself is exact");
        }
        other => panic!("a deeper rounding is MenuRounded, got {other:?}"),
    }
    let too_far = alter(11);
    assert!(compare(&rig.entry, &too_far.tree, 200, 1000, 10000).is_none(), "dev 11/200 > 0.05");
    assert_no_match(&rig, &too_far, "menu deviation above 5%");
}

// --- raw accuracy, inherited reasons, storage mode --------------------------------------------------------------------

/// Section 10.4 accuracy filter, raw: an entry at 0.005049 (displayed 50 bp) does not serve a 50 bp request as
/// covered (it is `Provisional`, no coverage claimed) and serves a 51 bp request `Exact`.
#[test]
fn raw_accuracy_0_005049_is_provisional_at_50_bp_and_exact_at_51() {
    let rig = CacheRig::with_entry(Scenario::new("flop_fast_v1", 100, 500, 5000), |e| e.exploitability_over_P = 0.005049);
    let at = |bp: u16| Scenario { target_bp: bp, ..rig.at(100, 500, 5000) }.query("oop");
    match rig.lookup(&at(50)) {
        Lookup::Provisional { hit, reasons } => assert_eq!((hit.coverage, reasons, hit.raw_exploitability_over_p), (None, vec![], 0.005049)),
        other => panic!("50 bp: Provisional, got {other:?}"),
    }
    match rig.lookup(&at(51)) {
        Lookup::Exact { hit } => assert_eq!(hit.coverage, Some(Coverage::Exact)),
        other => panic!("51 bp: Exact, got {other:?}"),
    }
}

/// Stored `ChartRounded`, `DeadlineBestSoFar` and `UnconditionedPriorStreet` all survive every lookup: at the entry's own
/// scale, beside a newly incurred `MenuRounded`, at a looser target, and on a `Provisional` hit.
#[test]
fn stored_chart_deadline_and_unconditioned_reasons_survive() {
    let stored = vec![
        ApproxReason::ChartRounded,
        ApproxReason::DeadlineBestSoFar { reached_bp: 40, target_bp: 30 },
        ApproxReason::UnconditionedPriorStreet { street: Street::Preflop, seat: OOP, cause: "no strategy".into() },
    ];
    let rig = CacheRig::with_entry(Scenario::new("flop_fast_v1", 100, 500, 5000), |e| e.reasons = stored.clone());
    match rig.lookup(&rig.query(100, 500, 5000, &[], "oop")) {
        Lookup::Approximate { reasons, hit } => {
            assert_eq!(reasons, stored);
            assert_eq!(hit.coverage, Some(Coverage::Approximate { reasons: stored.clone() }));
        }
        other => panic!("inherited reasons are Approximate, got {other:?}"),
    }
    let rounded = rig.query(103, 515, 5150, &[], "oop");
    let value = (100.0 * compare(&rig.entry, &rounded.tree, 103, 515, 5150).unwrap().max_dev) as f32;
    match rig.lookup(&rounded) {
        Lookup::Approximate { reasons, .. } => assert_eq!(reasons, [stored.clone(), vec![ApproxReason::MenuRounded { max_delta_pct: value }]].concat()),
        other => panic!("{other:?}"),
    }
    let looser = Scenario { target_bp: 100, ..rig.at(100, 500, 5000) }.query("oop");
    assert!(matches!(rig.lookup(&looser), Lookup::Approximate { reasons, .. } if reasons == stored));
    let tighter = Scenario { target_bp: 30, ..rig.at(100, 500, 5000) }.query("oop");
    assert!(matches!(rig.lookup(&tighter), Lookup::Provisional { reasons, .. } if reasons == stored));
}

/// An `i16` entry and an `f32` entry of the same solve serve identically -- same outcome, label, frequencies and EV --
/// and differ only in the disclosed source storage mode.
#[test]
fn i16_and_f32_entries_behave_identically() {
    let f32_rig = CacheRig::new("flop_fast_v1", 100, 500, 5000);
    let i16_rig = CacheRig::with_entry(Scenario::new("flop_fast_v1", 100, 500, 5000), |e| e.mode = "i16".into());
    for (p, eff, cap, history, actor) in [(100, 500, 5000, vec![], "oop"), (103, 515, 5150, vec![], "oop"), (100, 500, 5000, vec![Action::Bet { to: 50 }], "ip")] {
        let (a, b) = (f32_rig.lookup(&f32_rig.query(p, eff, cap, &history, actor)), i16_rig.lookup(&i16_rig.query(p, eff, cap, &history, actor)));
        let (ha, hb) = match (&a, &b) {
            (Lookup::Exact { hit: x }, Lookup::Exact { hit: y }) => (x, y),
            (Lookup::Approximate { hit: x, reasons: rx }, Lookup::Approximate { hit: y, reasons: ry }) if rx == ry => (x, y),
            other => panic!("{p}/{eff}: the same outcome, got {other:?}"),
        };
        assert_eq!((ha.coverage.clone(), served(ha).probs.clone(), served(ha).ev_chips.clone()), (hb.coverage.clone(), served(hb).probs.clone(), served(hb).ev_chips.clone()));
        assert_eq!((ha.source_mode.as_str(), hb.source_mode.as_str(), hb.solution.mode.as_str()), ("f32", "i16", "i16"));
        assert!(hb.notes.iter().any(|n| n.contains("i16")));
    }
}

/// The lookup budget the rig uses is generous (never a tight one): only `Cache::lookup`'s own 500 ms bound applies.
#[test]
fn the_rig_asks_with_a_generous_budget() {
    let rig = CacheRig::new("check_only_test_v1", 100, 500, 5000);
    let q = rig.query(100, 500, 5000, &[], "oop");
    assert_eq!(q.budget, BUDGET);
    assert!(BUDGET > cache::lookup::LOOKUP_BOUND);
    assert_eq!((q.actor.as_str(), rig.scenario.oop, rig.scenario.ip), ("oop", OOP, IP));
    assert!(cache::storage::entry_path(rig.root(), rig.entry.key.digest()).is_file(), "the entry's cell is on disk in the rig's own directory");
}
