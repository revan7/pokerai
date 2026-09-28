//! P3.T19 -- the two spec section 13.3 engine goldens that plan 3 owns, `replay_weights_golden` and
//! `bet_translation_golden`, frozen from independent Python oracles (spec sections 8.3, 8.4, 9.2).
//!
//! Both fixtures under `golden/` are committed data written by `tools/gen_preflop_fixtures.py`,
//! whose oracles use their own card, combo and class indexing and scalar Bayesian arithmetic --
//! never this workspace's output. Each carries `schema_version: 1`, `synthetic: true`, its complete
//! inputs (the synthetic source bundles embedded byte for byte with their manifests, the hands, the
//! menus and legal actions), its `expected` values and its tolerances. This runner drives only
//! public functions -- Plan 1's `begin_hand`/`apply_action`/`set_board`, the store's own admission
//! (`checked_envelope`, `build_node_map`), `core_replay::replay`, `interpolate`,
//! `destination_map`/`legalize_row`, `mix_nodes` and Plan 2's `assemble::headline` with the engine's
//! `headline_source` -- and compares every number within the fixture's stated tolerance and every
//! enum tag, string and flag exactly. There is no update mode: a mismatch is a finding against one
//! side, never a reason to re-record the golden from this code.

use core_preflop::{
    build_node_map, checked_envelope, destination_map, interpolate, legalize_row, mix_nodes, BranchNode, BundleInfo, EvReference,
    ExpandedNode, PokerDataJson, PreflopSource, PreflopStore, SourceKind,
};
use core_replay::{marginal, posterior, replay, HistoryBranch, ReplayInput, ReplayOutput, SeatMass};
use engine::assemble::headline;
use engine::preflop::headline_source;
use proto::{combo_index, Action, ApproxReason, HandConfig, HandState, LegalAction, Seat, COMBOS};
use serde_json::{json, Value};

const REPLAY_GOLDEN: &str = include_str!("golden/replay_weights_golden.json");
const BET_GOLDEN: &str = include_str!("golden/bet_translation_golden.json");

// ---------------------------------------------------------------------------------------------
// Fixture reading and comparison.
// ---------------------------------------------------------------------------------------------

/// A golden fixture, with the two header fields every golden of this file carries.
fn load(text: &str, name: &str) -> Value {
    let v: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name} is not valid JSON: {e}"));
    assert_eq!(v["schema_version"], 1, "{name}: schema_version");
    assert_eq!(v["synthetic"], true, "{name}: the fixture declares itself synthetic");
    v
}

fn num(v: &Value, what: &str) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{what}: {v} is not a number"))
}

/// The fixture's own tolerance for `name`: positive and far below any quantity it guards.
fn tol(golden: &Value, name: &str) -> f64 {
    let t = num(&golden["tolerances"][name], name);
    assert!(t > 0.0 && t <= 1e-4, "tolerance {name} = {t} is not a small positive number");
    t
}

fn from<T: serde::de::DeserializeOwned>(v: &Value, what: &str) -> T {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{what}: {e} in {v}"))
}

fn close(actual: f64, expected: f64, tol: f64, what: &str) {
    assert!(actual.is_finite() && (actual - expected).abs() <= tol, "{what}: {actual} != {expected} (tolerance {tol:e})");
}

/// `actual` against `expected`, recursively: every number within `tol`; every string, bool, null,
/// array length and object key set exactly.
fn json_close(actual: &Value, expected: &Value, tol: f64, path: &str) {
    match (actual, expected) {
        (Value::Number(a), Value::Number(e)) => {
            close(a.as_f64().expect("a JSON number"), e.as_f64().expect("a JSON number"), tol, path)
        }
        (Value::Array(a), Value::Array(e)) => {
            assert_eq!(a.len(), e.len(), "{path}: length ({actual} against {expected})");
            for (i, (x, y)) in a.iter().zip(e).enumerate() {
                json_close(x, y, tol, &format!("{path}[{i}]"));
            }
        }
        (Value::Object(a), Value::Object(e)) => {
            assert_eq!(a.keys().collect::<Vec<_>>(), e.keys().collect::<Vec<_>>(), "{path}: keys ({actual} against {expected})");
            for (k, y) in e {
                json_close(&a[k], y, tol, &format!("{path}.{k}"));
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}

fn to_json<T: serde::Serialize>(x: &T) -> Value {
    serde_json::to_value(x).expect("a wire type serializes")
}

// ---------------------------------------------------------------------------------------------
// Inputs: the embedded synthetic sources and the hands, through the public admission paths.
// ---------------------------------------------------------------------------------------------

/// A store of the fixture's embedded sources, each admitted exactly as the loader admits a bundle
/// on disk: the manifest deserialized into `BundleInfo`, the envelope's exact bytes hash-checked,
/// decoded and validated by `checked_envelope`, the node map built by `build_node_map`. Never
/// added to any chart directory.
fn store(sources: &Value) -> PreflopStore {
    let bundles: Vec<Box<dyn PreflopSource>> = sources
        .as_array()
        .expect("sources is a list")
        .iter()
        .map(|s| {
            let info: BundleInfo = from(&s["manifest"], "source manifest");
            assert!(info.bundle_id.starts_with("golden_"), "a golden source is named as one: {}", info.bundle_id);
            let raw = s["nodes_json"].as_str().expect("nodes_json holds the envelope's exact bytes");
            let envelope = checked_envelope(&info, raw.as_bytes()).unwrap_or_else(|e| panic!("{} is not admitted: {e}", info.bundle_id));
            let nodes = build_node_map(&info, &envelope).unwrap_or_else(|e| panic!("{} builds no node map: {e}", info.bundle_id));
            Box::new(PokerDataJson { info, nodes }) as Box<dyn PreflopSource>
        })
        .collect();
    assert!(!bundles.is_empty(), "a golden hand has a source");
    PreflopStore::from_sources(bundles)
}

/// The fixture's hand through Plan 1's lifecycle: `begin_hand`, every recorded action by the seat
/// the model says is to act, then the board.
fn hand(input: &Value) -> HandState {
    let cfg: HandConfig = from(&input["config"], "config");
    let hero_cards = input["hero_cards"].as_str().map(|t| core_model::parse_hand(t).unwrap_or_else(|e| panic!("hero cards {t}: {e}")));
    let begin = core_model::BeginHand {
        hand_id: from(&input["hand_id"], "hand_id"),
        button: from(&input["button"], "button"),
        hero: from(&input["hero"], "hero"),
        dealt: from(&input["dealt"], "dealt"),
        stacks_start: from(&input["stacks_start"], "stacks_start"),
        hero_cards,
    };
    let mut state = core_model::begin_hand(&cfg, begin).unwrap_or_else(|e| panic!("the table: {e}"));
    for (i, step) in input["actions"].as_array().expect("actions is a list").iter().enumerate() {
        let seat: Seat = from(&step["seat"], "seat");
        let action: Action = from(&step["action"], "action");
        assert_eq!(state.derived.to_act, Some(seat), "action {i}: the recorded seat is the seat to act");
        state = core_model::apply_action(&state, action).unwrap_or_else(|e| panic!("action {i} ({action:?}) is legal: {e}"));
    }
    let board = input["board"].as_str().expect("board is a string");
    if !board.is_empty() {
        let cards = core_model::parse_cards(board).unwrap_or_else(|e| panic!("board {board}: {e}"));
        let ids: Vec<u8> = from(&input["board_cards"], "board_cards");
        assert_eq!(cards.iter().map(|c| c.0).collect::<Vec<_>>(), ids, "the board's card ids are the oracle's");
        state = core_model::set_board(&state, &cards).unwrap_or_else(|e| panic!("board {board}: {e}"));
    }
    state
}

/// `hand(input)` replayed from a baseline caller's store of `sources` (no snapshots, no engine
/// provenance).
fn run(input: &Value, sources: &Value) -> (HandState, ReplayOutput) {
    let state = hand(input);
    let store = store(sources);
    let out = replay(ReplayInput { cfg: &state.config, state: &state, store: &store, snapshots: &[], missing: &[] });
    (state, out)
}

// ---------------------------------------------------------------------------------------------
// Replay comparisons.
// ---------------------------------------------------------------------------------------------

/// Every seat's published range, all 1326 entries, against the oracle's `f32`-converted vectors.
fn assert_ranges(out: &ReplayOutput, expected: &Value, tol: f64, what: &str) {
    let expected = expected.as_array().expect("ranges is a list");
    assert_eq!((out.ranges.len(), expected.len()), (6, 6), "{what}: six seats");
    for (seat, (got, want)) in out.ranges.iter().zip(expected).enumerate() {
        match (got, want.as_array()) {
            (None, None) => assert!(want.is_null(), "{what}: seat {seat}"),
            (Some(r), Some(w)) => {
                assert_eq!(w.len(), COMBOS, "{what}: seat {seat} has a full vector");
                for (c, x) in w.iter().enumerate() {
                    close(f64::from(r.0[c]), num(x, what), tol, &format!("{what}: seat {seat} combo {c}"));
                }
            }
            _ => panic!("{what}: seat {seat}: range present {} but expected {}", got.is_some(), if want.is_null() { "none" } else { "one" }),
        }
    }
}

fn assert_logs(out: &ReplayOutput, expected: &Value, tol: f64, what: &str) {
    let want: Vec<f64> = from(expected, "log_reach");
    assert_eq!((out.log_reach.len(), want.len()), (6, 6), "{what}: six log_reach entries");
    for (seat, (got, want)) in out.log_reach.iter().zip(&want).enumerate() {
        close(*got, *want, tol, &format!("{what}: log_reach[{seat}]"));
    }
}

/// The branch list: count, order, live flags, `q` and every translated history.
fn assert_branches(out: &ReplayOutput, expected: &Value, tol: f64, what: &str) {
    let want = expected.as_array().expect("branches is a list");
    assert_eq!(out.branches.len(), want.len(), "{what}: branch count");
    for (k, (b, w)) in out.branches.iter().zip(want).enumerate() {
        assert!(!b.residual && b.stopped.is_none(), "{what}: branch {k} is live: residual {} stopped {:?}", b.residual, b.stopped);
        close(b.q, num(&w["q"], what), tol, &format!("{what}: branch {k} q"));
        let translated: Vec<(Seat, Action)> = from(&w["translated"], "translated");
        assert_eq!(b.translated, translated, "{what}: branch {k} translated history");
    }
}

fn assert_reasons(reasons: &[ApproxReason], expected: &Value, tol: f64, what: &str) {
    json_close(&to_json(&reasons), expected, tol, &format!("{what}: reasons"));
}

// ---------------------------------------------------------------------------------------------
// replay_weights_golden (spec section 13.3: three-seat preflop history; 1326 vectors, log_reach).
// ---------------------------------------------------------------------------------------------

#[test]
fn replay_weights_golden() {
    let v = load(REPLAY_GOLDEN, "replay_weights_golden");
    let expected = v["expected"]["ranges"].as_array().unwrap();
    assert_eq!(expected.len(), 6);
    for r in expected.iter().filter(|r| !r.is_null()) {
        assert_eq!(r.as_array().unwrap().len(), 1326);
    }
    assert_eq!(v["input"]["dealt"], json!([0, 1, 2]));

    // The uniform line: BTN raises to the source's 2.5 bb, SB folds, BB calls; flop Kh7d2c.
    let input = &v["input"];
    let want = &v["expected"];
    let (state, out) = run(input, &input["sources"]);
    assert_eq!(out.unsupported, None, "the synthetic source answers every node");
    assert_ranges(&out, &want["ranges"], tol(&v, "range"), "uniform line");
    assert_logs(&out, &want["log_reach"], tol(&v, "log_reach"), "uniform line");
    assert_branches(&out, &want["branches"], tol(&v, "q"), "uniform line");
    close(out.branches[0].q, num(&want["q"], "q"), tol(&v, "q"), "uniform line: q");
    assert_reasons(&out.reasons, &want["reasons"], tol(&v, "reason"), "uniform line");
    // The SB folded: it keeps its posterior vector and is listed among the folded ranges.
    let folded: Vec<u8> = from(&want["folded"], "folded");
    assert_eq!((0..6u8).filter(|&i| state.derived.folded[usize::from(i)] && state.dealt.contains(&Seat(i))).collect::<Vec<_>>(), folded);
    let folded_ranges: Vec<_> = folded.iter().map(|&i| out.ranges[usize::from(i)].clone().expect("a folded dealt seat keeps its range")).collect();
    assert_eq!(out.folded_ranges, folded_ranges, "folded_ranges holds the folded seats' published ranges");
    // Hero's cards never enter a public range: every seat keeps weight on hero's own combo.
    let cards = state.hero_cards.expect("the fixture deals hero cards");
    let hero_combo = usize::from(combo_index(cards[0], cards[1]));
    for (seat, r) in out.ranges.iter().enumerate().filter_map(|(i, r)| r.as_ref().map(|r| (i, r))) {
        assert!(r.0[hero_combo] > 0.0, "seat {seat}'s public range keeps hero's combo");
    }

    // The off-menu line: BTN's raise sits between the source menu's two sizes, then the SB folds;
    // the BB is to act preflop.
    let off = &v["offmenu"];
    let input = &off["input"];
    let want = &off["expected"];
    let (state, out) = run(input, &input["sources"]);
    assert_eq!(state.derived.to_act, Some(Seat(2)), "the BB is to act");
    assert_eq!(out.unsupported, None);
    // f from the public interpolation at the replay's own source-parent pot fractions (the same
    // fractions the replay discloses in its BetTranslation reason, checked below).
    let s = num(&input["pot_fractions"]["s"], "s");
    let menu: Vec<f64> = from(&input["pot_fractions"]["menu"], "menu");
    let t = interpolate(s, &menu.iter().copied().enumerate().collect::<Vec<_>>()).expect("s lies between the menu sizes");
    assert_eq!(t.choices.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 1], "the observed raise is split over both sizes");
    let f_tol = tol(&v, "f");
    let f: Vec<f64> = from(&want["f"], "f");
    for (k, (got, want)) in t.choices.iter().map(|c| c.1).zip(&f).enumerate() {
        close(got, *want, f_tol, &format!("f[{k}]"));
    }
    close(t.choices[0].1, 81.0 / 173.0, f_tol, "f_A");
    close(t.choices[1].1, 92.0 / 173.0, f_tol, "f_B");
    assert_branches(&out, &want["branches"], tol(&v, "q"), "off-menu line");
    let q: Vec<f64> = from(&want["q"], "q");
    for (k, b) in out.branches.iter().enumerate() {
        close(b.q, q[k], tol(&v, "q"), &format!("off-menu line: q[{k}]"));
    }
    assert_ranges(&out, &want["ranges"], tol(&v, "range"), "off-menu line");
    assert_logs(&out, &want["log_reach"], tol(&v, "log_reach"), "off-menu line");
    // BTN's full marginal: the published range is the oracle's normalized marginal (above), and the
    // rescaled marginal times exp(log_reach) is the oracle's un-normalized one.
    let raw: Vec<f64> = from(&want["btn_marginal"], "btn_marginal");
    let scale = out.log_reach[0].exp();
    for (c, (got, want)) in marginal(&out.branches, Seat(0)).iter().zip(&raw).enumerate() {
        close(got * scale, *want, tol(&v, "range"), &format!("off-menu line: BTN marginal combo {c}"));
    }
    // BTN's per-combo branch posteriors at the three named combos.
    let named = want["btn_posterior"].as_object().expect("named posteriors");
    assert_eq!(named.keys().map(String::as_str).collect::<Vec<_>>(), vec!["72o", "AA", "AKs"]);
    for (name, entry) in named {
        let cards = core_model::parse_hand(entry["cards"].as_str().expect("cards")).expect("a hand");
        let combo = usize::from(combo_index(cards[0], cards[1]));
        assert_eq!(combo, from::<usize>(&entry["combo"], "combo"), "{name}: the oracle's combo index");
        assert_eq!(usize::from(proto::class_of(combo as u16)), from::<usize>(&entry["class"], "class"), "{name}: the oracle's class");
        let pi: Vec<f64> = from(&entry["posterior"], "posterior");
        for (k, (got, want)) in posterior(&out.branches, Seat(0), combo).iter().zip(&pi).enumerate() {
            close(*got, *want, tol(&v, "posterior"), &format!("BTN posterior {name} branch {k}"));
        }
    }
    // Cross-seat evidence: the BB has not acted, so its branch posterior is q_k / sum_j q_j for
    // every combo.
    let bb: Vec<f64> = from(&want["bb_posterior"], "bb_posterior");
    for c in 0..COMBOS {
        for (k, (got, want)) in posterior(&out.branches, Seat(2), c).iter().zip(&bb).enumerate() {
            close(*got, *want, tol(&v, "posterior"), &format!("BB posterior combo {c} branch {k}"));
        }
    }
    assert_reasons(&out.reasons, &want["reasons"], tol(&v, "reason"), "off-menu line");
}

// ---------------------------------------------------------------------------------------------
// bet_translation_golden (spec section 13.3 plus the plan 3 Task 19 legal-move, headline and
// prominence rows).
// ---------------------------------------------------------------------------------------------

/// One destination-mapped source row: the legalized actions compared field by field, the notes
/// exactly, and the probability mass conserved.
fn check_legal_move(case: &Value, tol: f64) {
    let name = case["name"].as_str().expect("name");
    let legal: Vec<LegalAction> = from(&case["legal"], "legal");
    let source = case["source"].as_array().expect("source rows");
    let actions: Vec<Action> = source.iter().map(|r| from(&r["action"], "source action")).collect();
    let probs: Vec<f32> = source.iter().map(|r| num(&r["probability"], name) as f32).collect();
    let evs: Vec<Option<f32>> = source.iter().map(|r| r["ev_chips"].as_f64().map(|x| x as f32)).collect();
    let (menu, map) = destination_map(&actions, &legal).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let mut notes = vec![];
    let advice = legalize_row(&menu, &map, &probs, &evs, &mut notes);
    assert_eq!(advice.unsupported, None, "{name}");
    let want = case["expected"]["actions"].as_array().expect("expected actions");
    assert_eq!(advice.actions.len(), want.len(), "{name}: menu length");
    for (i, (got, w)) in advice.actions.iter().zip(want).enumerate() {
        let what = format!("{name}: action {i}");
        assert_eq!(to_json(&got.action), w["action"], "{what}: action and amount");
        close(f64::from(got.probability), num(&w["probability"], &what), tol, &format!("{what}: probability"));
        match (got.ev_chips, w["ev_chips"].as_f64()) {
            (None, None) => assert!(w["ev_chips"].is_null(), "{what}"),
            (Some(g), Some(e)) => close(f64::from(g), e, tol, &format!("{what}: EV")),
            (g, _) => panic!("{what}: EV {g:?} against {}", w["ev_chips"]),
        }
        assert_eq!(to_json(&got.unavailable), w["unavailable"], "{what}: unavailable");
    }
    let wanted_notes: Vec<String> = from(&case["expected"]["notes"], "notes");
    assert_eq!(advice.notes, wanted_notes, "{name}: notes");
    let mass_in: f64 = probs.iter().map(|&p| f64::from(p)).sum();
    let mass_out: f64 = advice.actions.iter().map(|a| f64::from(a.probability)).sum();
    close(mass_out, mass_in, tol, &format!("{name}: probability mass is conserved"));
}

/// Hero's current decision over the fixture's branches: `mix_nodes`, then the one headline rule
/// with the engine's source mapping.
fn check_assembly(case: &Value, tol: f64) {
    let name = case["name"].as_str().expect("name");
    let hero: Seat = from(&case["hero"], "hero");
    let hero_combo: usize = from(&case["hero_combo"], "hero_combo");
    let bb_chips: u32 = from(&case["bb_chips"], "bb_chips");
    let branches: Vec<HistoryBranch> = case["branches"]
        .as_array()
        .expect("branches")
        .iter()
        .map(|b| {
            let zero: Vec<usize> = from(&b["zero_combos"], "zero_combos");
            let mass = (0..COMBOS).map(|c| if zero.contains(&c) { 0.0 } else { 1.0 }).collect();
            HistoryBranch {
                id: from(&b["id"], "id"),
                parent: None,
                split_by: None,
                translated: vec![],
                q: num(&b["q"], "q"),
                residual: from(&b["residual"], "residual"),
                stopped: from(&b["stopped"], "stopped"),
                seats: vec![SeatMass { seat: hero, node: None, mass }],
            }
        })
        .collect();
    let mut kinds: Vec<(SourceKind, EvReference)> = vec![];
    let nodes: Vec<BranchNode> = case["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|n| {
            let node = (!n["node"].is_null()).then(|| {
                let node = &n["node"];
                let actions: Vec<Action> = from(&node["actions"], "node actions");
                let probs: Vec<f32> = from::<Vec<f64>>(&node["probs"], "node probs").into_iter().map(|p| p as f32).collect();
                let evs: Vec<Option<f32>> = from::<Vec<Option<f64>>>(&node["evs"], "node evs").into_iter().map(|e| e.map(|x| x as f32)).collect();
                let source: SourceKind = from(&node["source"], "source");
                let ev_reference: EvReference = from(&node["ev_reference"], "ev_reference");
                kinds.push((source, ev_reference));
                ExpandedNode {
                    actor: hero,
                    actions,
                    probs: vec![probs; COMBOS],
                    ev_chips: vec![evs; COMBOS],
                    available: vec![true; COMBOS],
                    ev_reference,
                    source,
                }
            });
            BranchNode { branch_id: from(&n["branch_id"], "branch_id"), node, key: from(&n["key"], "key"), ..Default::default() }
        })
        .collect();
    let mixed = mix_nodes(&branches, &nodes, hero, hero_combo, bb_chips);
    let want = &case["expected"];
    // The headline, from the one rule, with the source mapping the engine uses for this node.
    kinds.dedup();
    assert!(kinds.len() <= 1, "{name}: one source kind and reference per case");
    let mut actions = mixed.actions.clone();
    let label = kinds.first().and_then(|&(s, r)| headline(&mut actions, mixed.unresolved_mass, headline_source(s, r)));
    json_close(&to_json(&actions), &want["actions"], tol, &format!("{name}: actions"));
    json_close(&json!(label), &want["headline"], tol, &format!("{name}: headline label"));
    close(f64::from(mixed.unresolved_mass), num(&want["unresolved_mass"], name), tol, &format!("{name}: unresolved mass"));
    json_close(&to_json(&mixed.range_mix), &want["range_mix"], tol, &format!("{name}: range mix"));
    json_close(&to_json(&mixed.reasons), &want["reasons"], tol, &format!("{name}: reasons"));
    assert_eq!(mixed.notes, from::<Vec<String>>(&want["notes"], "notes"), "{name}: notes");
    assert_eq!(to_json(&mixed.unsupported), want["unsupported"], "{name}: unsupported");
    // Known frequencies plus the unresolved share are the whole posterior wherever hero has advice.
    if mixed.unsupported.is_none() {
        let known: f64 = mixed.actions.iter().map(|a| f64::from(a.frequency.expect("a supported combo has frequencies"))).sum();
        close(known + f64::from(mixed.unresolved_mass), 1.0, tol, &format!("{name}: frequencies plus unresolved mass"));
    }
}

#[test]
fn bet_translation_golden() {
    let v = load(BET_GOLDEN, "bet_translation_golden");

    // Section 8.4's likelihood interpolation: below, between, above with and without an all-in
    // size, a single size and two equal sizes.
    let rows = v["interpolation"].as_array().expect("interpolation rows");
    assert_eq!(
        rows.iter().map(|r| r["name"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["below", "between", "above_no_jam", "above_with_jam", "single", "equal"]
    );
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let menu: Vec<f64> = from(&row["menu"], "menu");
        let t = interpolate(num(&row["s"], name), &menu.iter().copied().enumerate().collect::<Vec<_>>()).unwrap_or_else(|| panic!("{name}: no interpolation"));
        let mut f = vec![0.0; menu.len()];
        for &(i, x) in &t.choices {
            f[i] += x;
        }
        let want: Vec<f64> = from(&row["f"], "f");
        assert_eq!(f.len(), want.len(), "{name}");
        for (i, (got, want)) in f.iter().zip(&want).enumerate() {
            close(*got, *want, tol(&v, "f"), &format!("{name}: f[{i}]"));
        }
        close(t.deviation, num(&row["deviation"], name), tol(&v, "deviation"), &format!("{name}: deviation"));
        assert_eq!(t.clamped, row["clamped"].as_bool().unwrap(), "{name}: clamped");
    }

    // BetTranslation prominence is d > 0.10: an observed raise whose deviation is exactly 0.10 is
    // not prominent, one just above is. Driven through the public replay, the one producer.
    let prominence = &v["prominence"];
    let cases = prominence["cases"].as_array().expect("prominence cases");
    assert_eq!(cases.iter().map(|c| c["expected_prominent"].as_bool().unwrap()).collect::<Vec<_>>(), vec![false, true]);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let (_, out) = run(&case["input"], &prominence["sources"]);
        assert_eq!(out.unsupported, None, "{name}");
        assert_reasons(&out.reasons, &case["expected"]["reasons"], tol(&v, "reason"), name);
        let flags: Vec<bool> = out.reasons.iter().filter_map(|r| match r { ApproxReason::BetTranslation { prominent, .. } => Some(*prominent), _ => None }).collect();
        assert_eq!(flags, vec![case["expected_prominent"].as_bool().unwrap()], "{name}: prominence");
    }

    // Legality after mapping: below-minimum raises, above-stack wagers, created and owned
    // destinations.
    let moves = v["legal_moves"].as_array().expect("legal-move rows");
    assert_eq!(moves.len(), 4);
    for case in moves {
        check_legal_move(case, tol(&v, "probability"));
    }

    // Branch-supported assembly and its headline: the T7 frequency headline, the chart and
    // unverified-source wordings, a complete-EV headline, no headline with unresolved mass, no node
    // anywhere, hero out of support with the range mix, and the residual share by cause (ruling
    // 19-I1: the cap residual's share is "cap", a missing node's is "missing node <key>").
    let assembly = v["assembly"].as_array().expect("assembly rows");
    assert_eq!(assembly.len(), 9);
    for case in assembly {
        check_assembly(case, tol(&v, "probability"));
    }
}
