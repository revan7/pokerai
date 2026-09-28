//! P3.T17: the engine's chart and EV-bearing preflop decision path (spec 5 steps 5-6, spec 6's preflop rows, spec 8.3
//! and 8.4, spec 4.4's one headline rule).
//!
//! Every request here is admitted and served through Plan 2's own surfaces (`LiveRequest::admitted`,
//! `serve_request_with`, `Engine::with_core_and_seams`) on a fake clock and a scripted worker that records every message
//! the engine sends; every rig checks, when it is torn down, that no `solve` was ever sent. The equity routine is a
//! stub (`ServeSeams::equity`) that records what it was asked and answers at once, so each test ends deterministically.
//!
//! Sources: the committed chart bundles at every depth `fixtures/charts/sources.manifest.json` lists `available`
//! (an `unsupported` depth is skipped), and the committed synthetic PokerData bundle (`fixtures/preflop/synthetic_v2`)
//! only where a test injects it explicitly; the multi-size shapes spec 8.4 needs (splits, partial node presence, the
//! cap) are in-memory nodes under the synthetic manifest, exactly as `core-replay`'s own tests build them. A missing
//! fixture fails the test (standing ruling (e)).

use core_preflop::{
    build_node_map, checked_envelope, load_bundle, node_key, BundleInfo, EvReference, PokerDataJson, PreflopNode, PreflopNodeKey, PreflopSource,
    PreflopStep, PreflopStore, SourceKind,
};
use engine::clock::Clock;
use engine::core::EngineCore;
use engine::identity::IdentityState;
use engine::log::{DecisionLog, DecisionRecord};
use engine::serve::{serve_request_with, EquityRoutine, LiveRequest, ServeSeams};
use engine::testing::{FakeClock, FakeState, FakeWorker, Recorded, RecordingSink};
use engine::watchdog::SharedSink;
use proto::worker::EngineMessage;
use proto::{
    class_of, combo_index, Action, ApproxReason, Availability, Card, Coverage, DecisionIdentity, EquityEstimate, EquitySummary, HandConfig, HandState,
    LegalAction, Position, Rake, Range1326, Recommendation, RecommendationEvent, Seat, Street, Unavailable, UnsupportedReason,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------------------------
// The brief's Step 1 test, verbatim.
// ---------------------------------------------------------------------------------------------

#[test]
fn chart_headline_requires_complete_frequency_support(){
    use engine::assemble::{headline,HeadlineSource};
    use engine::preflop::headline_source;
    use core_preflop::{EvReference,SourceKind};
    use proto::{Action,ActionAdvice,Unavailable};
    // The preflop path chooses the source; the label itself is Plan 2's one rule.
    assert_eq!(headline_source(SourceKind::ChartTranscription,EvReference::Unverified),
        HeadlineSource::Chart);
    assert_eq!(headline_source(SourceKind::PokerDataJson,EvReference::Unverified),
        HeadlineSource::PokerDataUnverified);
    assert_eq!(headline_source(SourceKind::PokerDataJson,EvReference::DecisionIncrementalVerified),
        HeadlineSource::Solved);
    let mut actions=vec![
        ActionAdvice{action:Action::Fold,frequency:Some(0.2),ev_bb:None,
            unavailable:Some(Unavailable::ChartNoEv),headline:false},
        ActionAdvice{action:Action::Raise{to:5},frequency:Some(0.8),ev_bb:None,
            unavailable:Some(Unavailable::ChartNoEv),headline:false},
    ];
    assert_eq!(headline(&mut actions,0.,HeadlineSource::Chart).as_deref(),
        Some("highest-frequency chart action"));
    assert!(actions[1].headline);
    assert_eq!(headline(&mut actions,0.05,HeadlineSource::Chart),None);
    assert!(actions.iter().all(|a|!a.headline));
}

/// The other verified references map to the solved wording too (spec 8.3's four references; only `Unverified` is the
/// source-accuracy wording).
#[test]
fn every_verified_reference_maps_to_the_solved_headline_source() {
    use engine::assemble::HeadlineSource;
    use engine::preflop::headline_source;
    for reference in [EvReference::NetHandStartVerified, EvReference::AbsoluteStackVerified] {
        assert_eq!(headline_source(SourceKind::PokerDataJson, reference), HeadlineSource::Solved, "{reference:?}");
    }
    // A chart is a chart whatever its manifest says about a reference (charts never carry EV, spec 8.2).
    assert_eq!(headline_source(SourceKind::ChartTranscription, EvReference::DecisionIncrementalVerified), HeadlineSource::Chart);
}

// ---------------------------------------------------------------------------------------------
// Fixtures, tables and sources.
// ---------------------------------------------------------------------------------------------

/// Chips per big blind in every table below except the sub-chip one: blinds 5/10.
const UNIT: u32 = 10;
const SB: Seat = Seat(0);
const BB: Seat = Seat(1);
const UTG: Seat = Seat(2);
const HJ: Seat = Seat(3);
const CO: Seat = Seat(4);
const BTN: Seat = Seat(5);

/// A fresh temporary directory of this test process, removed (with everything in it) when the guard drops, a failing
/// assertion's unwinding included, so no run leaves a directory behind.
struct TempDir(PathBuf);

impl TempDir {
    /// `<temp>/pokerai_preflop_<tag>_<pid>`, emptied if a previous run left it; created when `create`.
    fn new(tag: &str, create: bool) -> TempDir {
        let dir = std::env::temp_dir().join(format!("pokerai_preflop_{tag}_{}", std::process::id()));
            if create {
            std::fs::create_dir_all(&dir).unwrap();
        }
        TempDir(dir)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// A config at `unit` chips per big blind whose rake is the synthetic bundle's own profile (5%, capped at 0.5 bb, no
/// flop no drop), so a symmetric 100 bb six-handed table maps with no reason at all.
fn config_at(unit: u32) -> HandConfig {
    HandConfig {
        config_revision: 1,
        sb_chips: unit / 2,
        bb_chips: unit,
        straddle: None,
        rake: Rake::PotRake { rate: 0.05, cap_mchips: unit * 500, no_flop_no_drop: true },
        chip_label: "$1".into(),
    }
}

/// A six-max table of `stacks_bb` big blinds at `unit` chips per big blind; seat 5 holds the button, so SB 0, BB 1,
/// UTG 2, HJ 3, CO 4, BTN 5.
fn table_at(unit: u32, stacks_bb: u32, hero: Seat, cards: &str) -> HandState {
    core_model::begin_hand(
        &config_at(unit),
        core_model::BeginHand {
            hand_id: 1,
            button: BTN,
            hero,
            dealt: (0..6).map(Seat).collect(),
            stacks_start: vec![stacks_bb * unit; 6],
            hero_cards: Some(core_model::parse_hand(cards).expect("a hand")),
        },
    )
    .expect("the model admits the table")
}

fn table(stacks_bb: u32, hero: Seat, cards: &str) -> HandState {
    table_at(UNIT, stacks_bb, hero, cards)
}

fn act(state: &HandState, actions: &[Action]) -> HandState {
    let mut next = state.clone();
    for a in actions {
        next = core_model::apply_action(&next, *a).unwrap_or_else(|e| panic!("legal action {a:?}: {e}"));
    }
    next
}

/// Hero's combo index and 169-class.
fn hero_class(state: &HandState) -> (usize, usize) {
    let [a, b] = state.hero_cards.expect("hero's cards are on record at a decision");
    let c = combo_index(a, b);
    (usize::from(c), usize::from(class_of(c)))
}

/// The synthetic bundle's manifest, with `reference` in place of its own EV reference, and its validated node map,
/// built by the loader's own admission functions.
fn synthetic(reference: EvReference) -> Box<dyn PreflopSource> {
    let dir = fixtures().join("preflop/synthetic_v2");
    let mut info: BundleInfo = serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("the committed synthetic manifest")).expect("it deserializes");
    info.ev_reference = reference;
    let raw = std::fs::read(dir.join("nodes.json")).expect("the committed synthetic nodes");
    let envelope = checked_envelope(&info, &raw).expect("the committed bundle passes admission");
    let nodes = build_node_map(&info, &envelope).expect("the committed bundle builds its node map");
    Box::new(PokerDataJson { info, nodes })
}

/// `(depth_bb, bundle_id)` of every depth the acquisition record lists `available`; an `unsupported` depth is skipped.
fn available_chart_depths() -> Vec<(u32, String)> {
    let raw = std::fs::read(fixtures().join("charts/sources.manifest.json")).expect("the committed acquisition record");
    let record: serde_json::Value = serde_json::from_slice(&raw).expect("it is JSON");
    let depths: Vec<(u32, String)> = record["depths"]
        .as_array()
        .expect("a depths array")
        .iter()
        .filter(|d| d["status"] == "available")
        .map(|d| (d["depth_bb"].as_u64().expect("a depth") as u32, d["bundle_id"].as_str().expect("a bundle id").to_string()))
        .collect();
    assert!(!depths.is_empty(), "at least one chart depth ships");
    depths
}

/// Every available chart bundle, loaded read-only through the store's own validated loader.
fn charts() -> Vec<Box<dyn PreflopSource>> {
    let dir = fixtures().join("charts");
    available_chart_depths()
        .into_iter()
        .map(|(_, id)| load_bundle(&dir.join(format!("{id}.manifest.json")), &dir.join(format!("{id}.json"))).unwrap_or_else(|e| panic!("{id}: {e}")))
        .collect()
}

/// The raw chart JSON's weight of action `a` for `class` at the node whose history is empty (an RFI decision at the
/// first seat), read independently of the loader.
fn chart_root_weights(bundle_id: &str, class: usize) -> Vec<(String, Option<u64>, f64)> {
    let raw = std::fs::read(fixtures().join(format!("charts/{bundle_id}.json"))).expect("the committed chart");
    let chart: serde_json::Value = serde_json::from_slice(&raw).expect("it is JSON");
    let node = chart["nodes"].as_array().expect("nodes").iter().find(|n| n["history"].as_array().is_some_and(|h| h.is_empty())).expect("an RFI root");
    node["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .enumerate()
        .map(|(a, action)| (action["step"].as_str().unwrap().to_string(), action["to_bb_x1000"].as_u64(), node["weights"][a][class].as_f64().unwrap()))
        .collect()
}

// In-memory multi-size sources (spec 8.4's shapes), under the synthetic bundle's manifest: 100 bb, "5% cap 0.5bb".

type MemNodes = Vec<(Vec<(Position, PreflopStep)>, PreflopNode)>;

fn raise(to_bb_x1000: u32) -> PreflopStep {
    PreflopStep::Raise { to_bb_x1000 }
}

/// A class-major probability table for a node with `n` actions: every entry positive, distinct per `seed`, each
/// class's row summing to 1 (in `f32`).
fn mixed(seed: usize, n: usize) -> Vec<Vec<f32>> {
    (0..169)
        .map(|c| {
            let w: Vec<f32> = (0..n).map(|a| 1.0 + ((c * (a + seed + 1) + 3 * a + seed) % 7) as f32).collect();
            let t: f32 = w.iter().sum();
            w.iter().map(|x| x / t).collect()
        })
        .collect()
}

/// An in-memory node with no EV and no unreachable class.
fn mem_node(actor: Position, actions: Vec<PreflopStep>, seed: usize) -> PreflopNode {
    let n = actions.len();
    PreflopNode { actor, actions, probs: mixed(seed, n), ev_source_sb: None, unreachable: [false; 169], committed_by_actor_sb: 0.0, fold_wide_verified: false }
}

fn key(history: Vec<(Position, PreflopStep)>) -> String {
    node_key(&PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history })
}

/// A PokerData source holding exactly `nodes`, under the synthetic manifest (verified reference, no EV in any node).
fn mem_store(nodes: MemNodes) -> PreflopStore {
    let dir = fixtures().join("preflop/synthetic_v2");
    let info: BundleInfo = serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("the committed synthetic manifest")).expect("it deserializes");
    let nodes: BTreeMap<String, PreflopNode> = nodes.into_iter().map(|(history, node)| (key(history), node)).collect();
    PreflopStore::from_sources(vec![Box::new(PokerDataJson { info, nodes })])
}

/// UTG opens 2.5 or 3.5 bb; HJ faces each size with its own menu: 6 or 9 bb after 2.5, 7 or 12 bb after 3.5.
fn ladder_nodes() -> MemNodes {
    let hj = |sizes: &[u32]| [PreflopStep::Fold, PreflopStep::Call].into_iter().chain(sizes.iter().map(|&s| raise(s))).collect();
    vec![
        (vec![], mem_node(Position::Utg, vec![PreflopStep::Fold, raise(2500), raise(3500)], 1)),
        (vec![(Position::Utg, raise(2500))], mem_node(Position::Hj, hj(&[6000, 9000]), 2)),
        (vec![(Position::Utg, raise(3500))], mem_node(Position::Hj, hj(&[7000, 12000]), 3)),
    ]
}

/// [`ladder_nodes`] one level deeper (the CO faces each UTG/HJ size pair with two raise sizes), and the button's node
/// after every one of the eight UTG/HJ/CO size triples, so hero on the button has a node in every live branch.
fn capped_ladder_nodes() -> MemNodes {
    let facing = |sizes: &[u32]| [PreflopStep::Fold, PreflopStep::Call].into_iter().chain(sizes.iter().map(|&s| raise(s))).collect::<Vec<_>>();
    let mut nodes = ladder_nodes();
    let co_menus = [((2500, 6000), [15000, 25000]), ((2500, 9000), [18000, 27000]), ((3500, 7000), [16000, 24000]), ((3500, 12000), [15000, 30000])];
    for (seed, ((open, hj), co)) in co_menus.into_iter().enumerate() {
        let after = vec![(Position::Utg, raise(open)), (Position::Hj, raise(hj))];
        nodes.push((after.clone(), mem_node(Position::Co, facing(&co), 4 + seed)));
        for (k, size) in co.into_iter().enumerate() {
            let mut btn = after.clone();
            btn.push((Position::Co, raise(size)));
            nodes.push((btn, mem_node(Position::Btn, facing(&[60000]), 8 + 2 * seed + k)));
        }
    }
    nodes
}

// ---------------------------------------------------------------------------------------------
// The rig: Plan 2's engine test surfaces, a recording worker, a stub equity routine.
// ---------------------------------------------------------------------------------------------

/// What the stub equity routine was asked: hero's cards, hero's public range, the opponents' public ranges, the board.
#[derive(Clone, Debug)]
struct EquityCall {
    hero: Option<[Card; 2]>,
    hero_public: Range1326,
    opponents: Vec<(Seat, Range1326)>,
    board: Vec<Card>,
}

/// A stub equity routine: records its inputs and answers at once, `Ready` 0.5 for every opponent in both populations.
fn stub_equity(calls: Arc<Mutex<Vec<EquityCall>>>) -> EquityRoutine {
    Arc::new(move |_clock: &dyn Clock, hero: Option<[Card; 2]>, hero_public: &Range1326, opponents: &[(Seat, Range1326)], board: &[Card],
        _budget: std::time::Duration, _cancel: &std::sync::atomic::AtomicBool| {
        calls.lock().unwrap().push(EquityCall { hero, hero_public: hero_public.clone(), opponents: opponents.to_vec(), board: board.to_vec() });
        let ready = || EquityEstimate { value: Some(0.5), availability: Availability::Ready, method: Some(proto::EquityMethod::Exact) };
        EquitySummary {
            hero_combo_vs_each: opponents.iter().map(|(s, _)| (*s, ready())).collect(),
            hero_range_vs_each: opponents.iter().map(|(s, _)| (*s, ready())).collect(),
            per_pot_shares: vec![],
        }
    })
}

struct Rig {
    core: EngineCore,
    clock: Arc<FakeClock>,
    identity: Arc<Mutex<IdentityState>>,
    fake: Arc<Mutex<FakeState>>,
    sink: SharedSink,
    events: Arc<Mutex<Vec<Recorded>>>,
    calls: Arc<Mutex<Vec<EquityCall>>>,
    /// The rig's decision log directory, removed when the rig drops (declared last, so after the core).
    log_dir: TempDir,
}

/// An engine core on a scripted worker with nothing scripted (a `solve` would hang, and is recorded), a fake clock, a
/// decision log of its own, and `store` as its loaded preflop store.
fn rig(name: &str, store: PreflopStore) -> Rig {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    identity.lock().unwrap().set_config();
    let (worker, fake) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![]);
    let log_dir = TempDir::new(name, false);
    let mut core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&log_dir));
    core.preflop = Arc::new(store);
    let (sink, events) = RecordingSink::new(clock.clone(), Some(fake.clone()));
    Rig { core, clock, identity, fake, sink: Arc::new(Mutex::new(Box::new(sink))), events, calls: Arc::default(), log_dir }
}

/// A new decision (a new hand, as far as the identity goes), admitted at the fake clock's now.
fn admit(r: &Rig, state: &HandState) -> LiveRequest {
    let id = { let mut s = r.identity.lock().unwrap(); s.begin_hand(); s.next_decision().unwrap() };
    LiveRequest::admitted(&r.core, id, state.clone(), r.clock.now_ms(), r.sink.clone())
}

fn serve_admitted(r: &mut Rig, req: LiveRequest) -> DecisionIdentity {
    let id = req.identity.clone();
    serve_request_with(&mut r.core, req, ServeSeams { equity: Some(stub_equity(r.calls.clone())), ..ServeSeams::default() });
    id
}

fn serve(r: &mut Rig, state: &HandState) -> DecisionIdentity {
    let req = admit(r, state);
    serve_admitted(r, req)
}

/// What a rig recorded, read once its core is torn down (every `fast-path` thread joined, so every `Equity` is in).
struct Served {
    events: Vec<Recorded>,
    calls: Vec<EquityCall>,
    records: Vec<DecisionRecord>,
}

/// Tears the rig's core down and reads what it recorded. Every preflop path sends no `solve` (the only messages a
/// preflop-only rig may have sent are the teardown's `shutdown`).
fn finish(mut r: Rig) -> Served {
    r.core.shutdown();
    let sent = r.fake.lock().unwrap().sent.clone();
    assert!(!sent.iter().any(|m| matches!(m, EngineMessage::Solve(_))), "a preflop path sent a solve: {sent:?}");
    assert!(sent.iter().all(|m| matches!(m, EngineMessage::Shutdown { .. })), "only the teardown's shutdown reached the worker: {sent:?}");
    let records = std::fs::read_to_string(r.log_dir.join("decisions.jsonl")).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default();
    Served { events: r.events.lock().unwrap().clone(), calls: r.calls.lock().unwrap().clone(), records }
}

fn kinds(events: &[Recorded]) -> Vec<&'static str> {
    events
        .iter()
        .map(|r| match r.event {
            RecommendationEvent::Fast(_) => "Fast",
            RecommendationEvent::Final(_) => "Final",
            RecommendationEvent::Equity { .. } => "Equity",
            RecommendationEvent::Progress { .. } => "Progress",
            RecommendationEvent::Provisional(_) => "Provisional",
            RecommendationEvent::NoDecision { .. } => "NoDecision",
        })
        .collect()
}

/// The one `Final` of decision `id`.
fn the_final(served: &Served, id: &DecisionIdentity) -> Recommendation {
    let finals: Vec<Recommendation> =
        served.events.iter().filter_map(|r| match &r.event { RecommendationEvent::Final(f) if f.identity == *id => Some(f.clone()), _ => None }).collect();
    assert_eq!(finals.len(), 1, "one Final per request: {:?}", kinds(&served.events));
    finals.into_iter().next().unwrap()
}

/// The `Fast` of decision `id`.
fn the_fast(served: &Served, id: &DecisionIdentity) -> Recommendation {
    served
        .events
        .iter()
        .find_map(|r| match &r.event { RecommendationEvent::Fast(f) if f.identity == *id => Some(f.clone()), _ => None })
        .expect("the request's Fast")
}

fn reasons(rec: &Recommendation) -> Vec<ApproxReason> {
    match &rec.coverage {
        Coverage::Exact => vec![],
        Coverage::Approximate { reasons } => reasons.clone(),
        Coverage::Unsupported { partial, .. } => partial.clone(),
    }
}

fn headline_note(rec: &Recommendation) -> Option<String> {
    rec.assumptions.notes.iter().find_map(|n| n.strip_prefix("headline: ").map(str::to_string))
}

/// Whether `action` is one of the live tree's legal actions: a wager inside its legal interval (spec 8.4's
/// legality after mapping), the exact all-in, or a like-named fold/check/call.
fn is_legal(action: &Action, legal: &[LegalAction]) -> bool {
    legal.iter().any(|l| match (action, l) {
        (Action::Fold, LegalAction::Fold) | (Action::Check, LegalAction::Check) | (Action::Call, LegalAction::Call { .. }) => true,
        (Action::Bet { to }, LegalAction::Bet { min_to, max_to }) | (Action::Raise { to }, LegalAction::Raise { min_to, max_to }) => to >= min_to && to <= max_to,
        (Action::AllIn { to }, LegalAction::AllIn { to: t }) => to == t,
        _ => false,
    })
}

fn frequencies(rec: &Recommendation) -> Vec<f64> {
    rec.actions.iter().map(|a| f64::from(a.frequency.unwrap_or_else(|| panic!("{:?} has a frequency", a.action)))).collect()
}

fn close(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: {a} != {b} (tolerance {tol})");
}

/// The replay the engine runs for `state`, run independently here for the branch weights a test checks against.
fn replayed(store: &PreflopStore, state: &HandState) -> core_replay::ReplayOutput {
    core_replay::replay(core_replay::ReplayInput { cfg: &state.config, state, store, snapshots: &[] })
}

// ---------------------------------------------------------------------------------------------
// Spec 6's preflop rows (the brief's Step 1 table).
// ---------------------------------------------------------------------------------------------

/// Row 1: a verified synthetic complete node. Hero (UTG, AKs) acts first at a symmetric 100 bb six-handed table whose
/// rake is the bundle's own profile: nothing is mapped and nothing is translated, so the `Final` is `Exact` with the
/// node's frequencies and its EVs normalized per Task 9 (decision-incremental: 1.84 source SB = 0.92 bb; the fold
/// exactly 0), headlined "highest EV" by Plan 2's rule. The request's events are its `Fast` (legal menu, pending), its
/// `Final` and then its `Equity`, over the replayed public ranges, none of which holds hero's cards.
#[test]
fn a_verified_complete_node_is_exact_with_normalized_evs() {
    let mut r = rig("verified", PreflopStore::from_sources(vec![synthetic(EvReference::DecisionIncrementalVerified)]));
    let state = table(100, UTG, "AsKs");
    let (hero_combo, class) = hero_class(&state);
    assert_eq!(class, 1, "AKs is the fixture's one mixing class");
    let id = serve(&mut r, &state);
    let served = finish(r);
    assert_eq!(kinds(&served.events), ["Fast", "Final", "Equity"], "Fast, then the Final, then the Equity");
    let fast = the_fast(&served, &id);
    assert_eq!((fast.phase, fast.coverage.clone()), (proto::Phase::Fast, Coverage::Exact));
    assert!(fast.actions.iter().all(|a| a.frequency.is_none() && a.ev_bb.is_none() && a.unavailable == Some(Unavailable::Pending)), "{:?}", fast.actions);
    let f = the_final(&served, &id);
    assert_eq!((f.phase, f.coverage.clone()), (proto::Phase::Final, Coverage::Exact), "{:?}", f.coverage);
    assert_eq!(f.actions.iter().map(|a| a.action).collect::<Vec<_>>(), [Action::Fold, Action::Raise { to: 25 }]);
    let freq = frequencies(&f);
    close(freq[0], 0.65, 1e-6, "fold");
    close(freq[1], 0.35, 1e-6, "raise");
    assert_eq!(f.actions[0].ev_bb.map(f32::to_bits), Some(0.0f32.to_bits()), "the fold is exactly 0");
    close(f64::from(f.actions[1].ev_bb.expect("the raise has an EV")), 1.84 * 0.5 * f64::from(UNIT) / f64::from(UNIT), 1e-5, "raise EV in bb");
    assert!(f.actions.iter().all(|a| a.unavailable.is_none()), "{:?}", f.actions);
    assert_eq!(f.unresolved_mass, 0.0);
    assert!(f.range_mix.is_some(), "the range mix is present whenever a node strategy exists");
    assert!(f.actions[1].headline && !f.actions[0].headline);
    assert_eq!(headline_note(&f).as_deref(), Some("highest EV"));
    assert!(f.assumptions.source.contains("synthetic_v2_100bb"), "{}", f.assumptions.source);
    assert_eq!((f.assumptions.source_accuracy.as_str(), f.assumptions.source_granularity.as_str()), ("unverified", "169-class"));
    assert_eq!(f.assumptions.ranges_used.iter().map(|u| u.0).collect::<Vec<_>>(), (0..6).map(Seat).collect::<Vec<_>>(), "every seat still in the hand");
    assert!(f.assumptions.translations.is_empty() && f.assumptions.mappings.is_empty());
    // Equity over the replayed public ranges: hero's own combo keeps its public weight in every range (hero's cards never
    // enter a public range; only the equity routine conditions a private copy).
    assert_eq!(served.calls.len(), 1);
    let call = &served.calls[0];
    assert_eq!(call.hero, state.hero_cards);
    assert!(call.board.is_empty());
    assert_eq!(call.opponents.iter().map(|o| o.0).collect::<Vec<_>>(), [SB, BB, HJ, CO, BTN]);
    assert_eq!(call.hero_public.0[hero_combo], 1.0, "hero's combo in hero's public range");
    assert!(call.opponents.iter().all(|(_, range)| range.0[hero_combo] == 1.0), "hero's combo in every opponent's public range");
    // Logged once, as a Final that solved no street.
    assert_eq!(served.records.len(), 1);
    assert_eq!((served.records[0].street, served.records[0].coverage.clone(), served.records[0].street_violation), (Street::Preflop, Coverage::Exact, false));
}

/// Row 2: a synthetic bundle whose EV reference is unverified: frequencies, no EV anywhere, `EvReferenceUnverified`,
/// and the source wording of Plan 2's frequency headline on the highest-frequency action.
#[test]
fn an_unverified_reference_gives_frequencies_without_evs() {
    let mut r = rig("unverified", PreflopStore::from_sources(vec![synthetic(EvReference::Unverified)]));
    let state = table(100, UTG, "AsKs");
    let id = serve(&mut r, &state);
    let served = finish(r);
    let f = the_final(&served, &id);
    assert_eq!(f.coverage, Coverage::Approximate { reasons: vec![ApproxReason::EvReferenceUnverified] });
    let freq = frequencies(&f);
    close(freq[0], 0.65, 1e-6, "fold");
    close(freq[1], 0.35, 1e-6, "raise");
    assert!(f.actions.iter().all(|a| a.ev_bb.is_none() && a.unavailable == Some(Unavailable::NoEvReference)), "{:?}", f.actions);
    assert!(f.actions[0].headline && !f.actions[1].headline);
    assert_eq!(headline_note(&f).as_deref(), Some("highest-frequency source action, EV reference unverified"));
}

/// Row 3 at every available chart depth (Step 5): hero opens first at a table of exactly that depth, holding a hand
/// the chart raises and one it folds. Each `Final` holds the chart's own frequencies for hero's class (read from the
/// raw chart independently), summing to 1, every wager a legal chip size, `ChartRounded` (with the undocumented chart
/// rake disclosed as a mapping), no EV (`ChartNoEv`), and the chart headline on the highest-frequency action.
#[test]
fn a_chart_node_at_every_available_depth_gives_rounded_frequencies_without_ev() {
    for (depth, bundle_id) in available_chart_depths() {
        for cards in ["AhAd", "7h2c"] {
            let mut r = rig(&format!("chart_{depth}_{cards}"), PreflopStore::from_sources(charts()));
            let state = table(depth, UTG, cards);
            let (_, class) = hero_class(&state);
            let id = serve(&mut r, &state);
            let served = finish(r);
            let f = the_final(&served, &id);
            let what = format!("{bundle_id} {cards}");
            assert!(f.assumptions.source.contains(&bundle_id), "{what}: {}", f.assumptions.source);
            let chart = chart_root_weights(&bundle_id, class);
            assert_eq!(f.actions.len(), chart.len(), "{what}");
            for (advice, (step, size, weight)) in f.actions.iter().zip(&chart) {
                let want = match (step.as_str(), size) {
                    ("fold", _) => Action::Fold,
                    ("call", _) => Action::Call,
                    // Task 9's chip conversion: thousandths of a source unit, rounded half up to the chip.
                    ("raise", Some(s)) => Action::Raise { to: (*s as u32 * UNIT + 500) / 1000 },
                    other => panic!("{what}: an RFI chart step {other:?}"),
                };
                assert_eq!(advice.action, want, "{what}");
                close(f64::from(advice.frequency.expect("a chart frequency")), *weight, 1e-6, &what);
                assert!(advice.ev_bb.is_none() && advice.unavailable == Some(Unavailable::ChartNoEv), "{what}: {advice:?}");
                assert!(is_legal(&advice.action, &state.derived.legal), "{what}: {:?} is not legal in {:?}", advice.action, state.derived.legal);
            }
            close(frequencies(&f).iter().sum(), 1.0, 1e-6, &format!("{what}: the frequencies form a distribution"));
            let rs = reasons(&f);
            assert!(matches!(f.coverage, Coverage::Approximate { .. }) && rs.contains(&ApproxReason::ChartRounded), "{what}: {:?}", f.coverage);
            assert!(rs.iter().any(|x| matches!(x, ApproxReason::RakeProfileMapped { used, .. } if used == "undocumented chart rake")), "{what}: {rs:?}");
            assert!(!rs.contains(&ApproxReason::EvReferenceUnverified), "{what}: a chart is not an unverified PokerData reference");
            assert!(!rs.iter().any(|x| matches!(x, ApproxReason::DepthBucket { .. })), "{what}: the table is exactly the chart's depth");
            assert_eq!(headline_note(&f).as_deref(), Some("highest-frequency chart action"), "{what}");
            let best = f.actions.iter().position(|a| a.headline).expect("a headline action");
            assert!(f.actions.iter().all(|a| a.frequency <= f.actions[best].frequency), "{what}: the headline is the highest frequency");
        }
    }
}

/// Row 4: hero's node is present in only some of hero's positive-posterior branches. UTG opens 3 bb between the
/// source's 2.5 and 3.5 bb (a split, `BetTranslation`); hero (HJ) has a node after 2.5 bb only. The known frequencies
/// are that branch's posterior times its node's, the rest is `unresolved_mass` (the 3.5 bb branch's posterior, with
/// `BranchResidual{cause: "missing node <key>"}` naming its missing key and the share note), there is no EV and no
/// headline of any kind (spec 4.4 rule 3). The `Fast` already carries the replay's translation.
#[test]
fn a_node_in_only_some_positive_branches_gives_known_frequencies_and_the_unresolved_share() {
    let missing_after_35 = key(vec![(Position::Utg, raise(3500))]);
    let nodes: MemNodes = ladder_nodes().into_iter().filter(|(h, _)| key(h.clone()) != missing_after_35).collect();
    let hj_node = nodes.iter().find(|(h, _)| *h == vec![(Position::Utg, raise(2500))]).expect("HJ's node after 2.5 bb").1.clone();
    let state = act(&table(100, HJ, "AhKd"), &[Action::Raise { to: 30 }]);
    let (_, class) = hero_class(&state);
    let out = replayed(&mem_store(nodes.clone()), &state);
    assert_eq!(out.branches.len(), 2, "the off-menu open split the list");
    let q = |to: u32| out.branches.iter().find(|b| b.translated == vec![(UTG, Action::Raise { to })]).expect("a branch per mapped size").q;
    let (q25, q35) = (q(25), q(35));
    let unresolved = q35 / (q25 + q35); // HJ has not acted: hero's posterior is the branch weights'
    let mut r = rig("partial", mem_store(nodes));
    let id = serve(&mut r, &state);
    let served = finish(r);
    let fast = the_fast(&served, &id);
    assert!(reasons(&fast).iter().any(|x| matches!(x, ApproxReason::BetTranslation { street: Street::Preflop, seat, .. } if *seat == UTG)), "{:?}", fast.coverage);
    let f = the_final(&served, &id);
    assert!(matches!(f.coverage, Coverage::Approximate { .. }), "{:?}", f.coverage);
    close(f64::from(f.unresolved_mass), unresolved, 1e-6, "unresolved mass");
    assert_eq!(f.actions.iter().map(|a| a.action).collect::<Vec<_>>(), [Action::Fold, Action::Call, Action::Raise { to: 60 }, Action::Raise { to: 90 }]);
    let row = &hj_node.probs[class];
    let row_sum: f64 = row.iter().map(|p| f64::from(*p)).sum();
    for (advice, p) in f.actions.iter().zip(row) {
        close(f64::from(advice.frequency.unwrap()), (1.0 - unresolved) * f64::from(*p) / row_sum, 1e-6, &format!("{:?}", advice.action));
        assert!(advice.ev_bb.is_none() && matches!(advice.unavailable, Some(Unavailable::BranchSupportIncomplete { .. })), "{advice:?}");
        assert!(!advice.headline);
    }
    close(frequencies(&f).iter().sum::<f64>() + f64::from(f.unresolved_mass), 1.0, 1e-6, "sum(frequency) + unresolved_mass = 1");
    assert_eq!(headline_note(&f), None, "no headline while unresolved_mass > 0");
    let rs = reasons(&f);
    let residuals: Vec<&ApproxReason> = rs.iter().filter(|x| matches!(x, ApproxReason::BranchResidual { .. })).collect();
    assert_eq!(residuals.len(), 1, "{rs:?}");
    match residuals[0] {
        ApproxReason::BranchResidual { seat, residual_mass_pct, cause } => {
            assert_eq!(*seat, HJ);
            close(f64::from(*residual_mass_pct), 100.0 * unresolved, 1e-4, "residual share");
            assert!(cause.starts_with("missing node ") && cause.contains("3500"), "{cause}");
        }
        other => panic!("{other:?}"),
    }
    assert!(rs.iter().any(|x| matches!(x, ApproxReason::BetTranslation { seat, .. } if *seat == UTG)), "the replay's translation is inherited: {rs:?}");
    assert!(f.assumptions.notes.iter().any(|n| n.ends_with("% of the posterior has no strategy")), "{:?}", f.assumptions.notes);
    assert!(f.assumptions.translations.iter().all(|t| matches!(t, ApproxReason::BetTranslation { .. })) && !f.assumptions.translations.is_empty());
}

/// Row 5: hero's node is absent in every positive-posterior branch (both of HJ's nodes removed after the split
/// open): `Unsupported{MissingPreflopNode}` with the heaviest branch's key (the 3.5 bb path), that exact key in the
/// assumptions too (spec 12, ruling 17-I2), the replay's reasons kept in `partial`, the legal intervals with no
/// advice, and equity only (the `Equity` event still follows).
#[test]
fn a_node_in_no_positive_branch_is_missing_with_the_inherited_reasons() {
    let nodes: MemNodes = ladder_nodes().into_iter().filter(|(h, _)| h.is_empty()).collect();
    let state = act(&table(100, HJ, "AhKd"), &[Action::Raise { to: 30 }]);
    let out = replayed(&mem_store(nodes.clone()), &state);
    let heaviest = out.branches.iter().max_by(|a, b| a.q.total_cmp(&b.q)).expect("two branches");
    assert_eq!(heaviest.translated, vec![(UTG, Action::Raise { to: 35 })], "the 3.5 bb branch is the heavier one");
    let mut r = rig("missing", mem_store(nodes));
    let id = serve(&mut r, &state);
    let served = finish(r);
    assert_eq!(kinds(&served.events), ["Fast", "Final", "Equity"], "equity only, after the Final");
    let f = the_final(&served, &id);
    let after_35 = key(vec![(Position::Utg, raise(3500))]);
    match &f.coverage {
        Coverage::Unsupported { reason: UnsupportedReason::MissingPreflopNode { key }, partial } => {
            assert_eq!(*key, after_35, "the heaviest branch's key");
            assert!(f.assumptions.notes.contains(&format!("missing preflop node: {key}")), "the key in the assumptions: {:?}", f.assumptions.notes);
            assert!(partial.iter().any(|x| matches!(x, ApproxReason::BetTranslation { seat, .. } if *seat == UTG)), "{partial:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(f.actions.len(), state.derived.legal.len());
    assert!(f.actions.iter().all(|a| a.frequency.is_none() && a.ev_bb.is_none() && a.unavailable == Some(Unavailable::NotEvaluated)), "{:?}", f.actions);
    assert_eq!((f.range_mix.clone(), f.unresolved_mass), (None, 0.0));
}

/// Row 6: hero's combo has zero weight at the node while hero's public range is positive. Hero (UTG, AA) opened although
/// the source never raises AA, then faces a raise: hero's posterior is all zero, so the `Final` is
/// `Unsupported{HeroComboOutOfSupport}` with the range-level mix as the only strategy output and no frequency or EV for
/// hero's combo. The same holds where hero's node marks hero's class explicitly unreachable (the synthetic fixture's
/// 22 at HJ's node after an open).
#[test]
fn a_zero_hero_combo_with_a_positive_public_range_is_out_of_support_with_the_range_mix() {
    let aa = 0usize;
    let mut root = mem_node(Position::Utg, vec![PreflopStep::Fold, raise(2500)], 1);
    root.probs[aa] = vec![1.0, 0.0];
    let fold_or_call = |actor: Position, seed: usize| mem_node(actor, vec![PreflopStep::Fold, PreflopStep::Call], seed);
    let mut history = vec![(Position::Utg, raise(2500))];
    let mut nodes: MemNodes = vec![(vec![], root), (history.clone(), mem_node(Position::Hj, vec![PreflopStep::Fold, raise(8000)], 2))];
    history.push((Position::Hj, raise(8000)));
    for (seed, position) in [Position::Co, Position::Btn, Position::Sb, Position::Bb].into_iter().enumerate() {
        nodes.push((history.clone(), fold_or_call(position, 3 + seed)));
        history.push((position, PreflopStep::Fold));
    }
    nodes.push((history, mem_node(Position::Utg, vec![PreflopStep::Fold, PreflopStep::Call, raise(20000)], 9)));
    let state = act(&table(100, UTG, "AhAd"), &[Action::Raise { to: 25 }, Action::Raise { to: 80 }, Action::Fold, Action::Fold, Action::Fold, Action::Fold]);
    assert_eq!(hero_class(&state).1, aa);
    let unreachable = act(&table(100, HJ, "2c2d"), &[Action::Raise { to: 25 }]);
    assert_eq!(hero_class(&unreachable).1, 168, "22 is the synthetic fixture's unreachable class");
    for (what, store, state) in [("zero posterior", mem_store(nodes), state), ("unreachable class", PreflopStore::from_sources(vec![synthetic(EvReference::DecisionIncrementalVerified)]), unreachable)] {
        let mut r = rig(&format!("out_of_support_{}", what.replace(' ', "_")), store);
        let id = serve(&mut r, &state);
        let served = finish(r);
        let f = the_final(&served, &id);
        assert!(matches!(f.coverage, Coverage::Unsupported { reason: UnsupportedReason::HeroComboOutOfSupport, .. }), "{what}: {:?}", f.coverage);
        assert!(!f.actions.is_empty() && f.actions.iter().all(|a| a.frequency.is_none() && a.ev_bb.is_none() && a.unavailable == Some(Unavailable::HeroOutOfSupport) && !a.headline),
            "{what}: {:?}", f.actions);
        let mix = f.range_mix.clone().unwrap_or_else(|| panic!("{what}: the range mix is the only strategy output"));
        assert_eq!(mix.iter().map(|m| m.0).collect::<Vec<_>>(), f.actions.iter().map(|a| a.action).collect::<Vec<_>>(), "{what}");
        assert!(mix.iter().all(|(_, share)| (0.0..=1.0).contains(share)) && mix.iter().any(|(_, share)| *share > 0.0), "{what}: {mix:?}");
        assert_eq!(headline_note(&f), None, "{what}");
        assert!(kinds(&served.events).contains(&"Equity"), "{what}: equity is still shown");
    }
}

// ---------------------------------------------------------------------------------------------
// Beyond the table: inherited reasons, the cap, source precedence, the walk's own lookups, the claim.
// ---------------------------------------------------------------------------------------------

/// Step 5: a previous replay reason survives a current exact lookup. UTG opens 3 bb against a source that offers only
/// 2.5 bb (clamped: one branch, `BetTranslation`); hero's own node after the mapped open is found exactly, and its EVs
/// are complete for fold and call, yet the `Final` stays `Approximate` with the translation (reasons accumulate).
#[test]
fn a_previous_replay_reason_survives_a_current_exact_lookup() {
    let mut r = rig("inherited", PreflopStore::from_sources(vec![synthetic(EvReference::DecisionIncrementalVerified)]));
    let state = act(&table(100, HJ, "AsKs"), &[Action::Raise { to: 30 }]);
    let id = serve(&mut r, &state);
    let served = finish(r);
    let f = the_final(&served, &id);
    let rs = reasons(&f);
    assert!(matches!(f.coverage, Coverage::Approximate { .. }) && rs.iter().any(|x| matches!(x, ApproxReason::BetTranslation { seat, .. } if *seat == UTG)), "{:?}", f.coverage);
    assert_eq!(f.actions.iter().map(|a| a.action).collect::<Vec<_>>(), [Action::Fold, Action::Call, Action::Raise { to: 88 }]);
    assert_eq!(f.actions[0].ev_bb.map(f32::to_bits), Some(0.0f32.to_bits()));
    close(f64::from(f.actions[1].ev_bb.expect("the call's EV")), 0.92, 1e-5, "call EV");
    assert_eq!((f.actions[2].ev_bb, f.actions[2].unavailable.clone()), (None, Some(Unavailable::NoEvReference)), "no source EV for the 3bet");
    assert_eq!(headline_note(&f), None, "a solved source with an incomplete EV menu has no headline wording (spec 4.4)");
}

/// Item 5 of the dispatch (ruling 15-Q5 carry): the cap residual and the assembly's "missing node" residual are
/// different disclosures. Here UTG, HJ and CO each raise between two source sizes (8 branches, capped to 4 live plus
/// the residual) and hero (the button) has a node in every live branch: the only posterior without a strategy is the
/// residual's. It is disclosed exactly once, by the replay boundary's `BranchResidual{cause: "cap"}`; the assembly's
/// "missing node" wording, which would name a node that is present, is not added; hero's unresolved share stays on the
/// `Final` (`unresolved_mass` and its note), and there is no headline.
#[test]
fn a_cap_residual_alone_is_disclosed_once_as_the_cap() {
    let nodes = capped_ladder_nodes();
    let state = act(&table(100, BTN, "AhKd"), &[Action::Raise { to: 30 }, Action::Raise { to: 75 }, Action::Raise { to: 200 }]);
    let out = replayed(&mem_store(nodes.clone()), &state);
    assert_eq!((out.branches.iter().filter(|b| b.residual).count(), out.branches.iter().filter(|b| !b.residual).count()), (1, 4), "capped to 4 plus the residual");
    let total: f64 = out.branches.iter().map(|b| b.q).sum();
    let share = out.branches.iter().find(|b| b.residual).unwrap().q / total; // the button has not acted: hero's posterior
    let mut r = rig("cap", mem_store(nodes));
    let id = serve(&mut r, &state);
    let served = finish(r);
    let f = the_final(&served, &id);
    let rs = reasons(&f);
    let residuals: Vec<&ApproxReason> = rs.iter().filter(|x| matches!(x, ApproxReason::BranchResidual { .. })).collect();
    assert_eq!(residuals.len(), 1, "exactly one residual disclosure: {rs:?}");
    match residuals[0] {
        ApproxReason::BranchResidual { seat, residual_mass_pct, cause } => {
            assert_eq!((*seat, cause.as_str()), (BTN, "cap"));
            close(f64::from(*residual_mass_pct), 100.0 * share, 1e-4, "the cap share");
        }
        other => panic!("{other:?}"),
    }
    // One translation per branch that split at each raise (spec 8.4: each at its own mapped parent): 1, 2, then 4.
    let translated: Vec<Seat> = rs.iter().filter_map(|x| match x { ApproxReason::BetTranslation { seat, .. } => Some(*seat), _ => None }).collect();
    assert_eq!(translated, [UTG, HJ, HJ, CO, CO, CO, CO], "{rs:?}");
    close(f64::from(f.unresolved_mass), share, 1e-6, "hero's unresolved share is the residual's");
    close(frequencies(&f).iter().sum::<f64>() + f64::from(f.unresolved_mass), 1.0, 1e-6, "sum(frequency) + unresolved_mass = 1");
    assert!(f.assumptions.notes.iter().any(|n| n.ends_with("% of the posterior has no strategy")), "{:?}", f.assumptions.notes);
    assert!(f.assumptions.notes.iter().any(|n| n.contains("cap residual")), "the deduplication is disclosed: {:?}", f.assumptions.notes);
    assert!(f.actions.iter().all(|a| !a.headline) && headline_note(&f).is_none());
}

/// Spec 8.4's legality after mapping on hero's node: hero (UTG, AKs) holds 20 chips, so the source's 2.5 bb open (25
/// chips) is above the live maximum and its probability moves to the legal all-in, a destination the move created:
/// hero's advice is fold and all-in, the fold keeps its exact 0 EV, the all-in has no EV and says why
/// (`MovedProbability{from}`, the source action), and the move is noted. Hero's depth and the asymmetric stacks are
/// mapping reasons of hero's own lookup (spec 6 row 1: `Approximate` with them), disclosed as mappings.
#[test]
fn a_source_size_above_heros_stack_moves_to_the_legal_all_in() {
    let mut r = rig("legality", PreflopStore::from_sources(vec![synthetic(EvReference::DecisionIncrementalVerified)]));
    let state = core_model::begin_hand(
        &config_at(UNIT),
        core_model::BeginHand { hand_id: 1, button: BTN, hero: UTG, dealt: (0..6).map(Seat).collect(), stacks_start: vec![1000, 1000, 20, 1000, 1000, 1000],
            hero_cards: Some(core_model::parse_hand("AsKs").unwrap()) },
    )
    .expect("the model admits the table");
    // The source's 25-chip open is above every legal raise, and the all-in is legal.
    assert!(state.derived.legal.iter().all(|l| !matches!(l, LegalAction::Raise { max_to, .. } if *max_to >= 25)) && state.derived.legal.contains(&LegalAction::AllIn { to: 20 }),
        "{:?}", state.derived.legal);
    let id = serve(&mut r, &state);
    let f = the_final(&finish(r), &id);
    assert_eq!(f.actions.iter().map(|a| a.action).collect::<Vec<_>>(), [Action::Fold, Action::AllIn { to: 20 }]);
    assert!(f.actions.iter().all(|a| is_legal(&a.action, &state.derived.legal)));
    let freq = frequencies(&f);
    close(freq[0], 0.65, 1e-6, "fold");
    close(freq[1], 0.35, 1e-6, "the moved open");
    assert_eq!((f.actions[0].ev_bb.map(f32::to_bits), f.actions[0].unavailable.clone()), (Some(0.0f32.to_bits()), None));
    assert_eq!((f.actions[1].ev_bb, f.actions[1].unavailable.clone()), (None, Some(Unavailable::MovedProbability { from: Action::Raise { to: 25 } })));
    assert!(f.assumptions.notes.iter().any(|n| n.starts_with("Moved Raise { to: 25 } probability") && n.ends_with("to AllIn { to: 20 }")), "{:?}", f.assumptions.notes);
    let rs = reasons(&f);
    assert!(rs.iter().any(|x| matches!(x, ApproxReason::DepthBucket { seat, used_bb: 100, .. } if *seat == UTG)), "{rs:?}");
    assert!(f.assumptions.mappings.iter().any(|x| matches!(x, ApproxReason::DepthBucket { .. })), "{:?}", f.assumptions.mappings);
    assert_eq!(headline_note(&f), None, "a solved source without a complete EV menu has no headline wording");
}

/// Source precedence (spec 8.2, Step 5): PokerData over charts, and synthetic PokerData only when a test injects it.
/// With the charts alone the 100 bb RFI decision is the chart's; with the synthetic bundle injected beside them it is
/// the synthetic bundle's, `Exact` and EV-bearing.
#[test]
fn source_precedence_prefers_injected_pokerdata_over_charts() {
    let state = table(100, UTG, "AsKs");
    let mut with_charts = charts();
    let mut r = rig("charts_only", PreflopStore::from_sources(charts()));
    let id = serve(&mut r, &state);
    let chart_final = the_final(&finish(r), &id);
    assert!(reasons(&chart_final).contains(&ApproxReason::ChartRounded) && !chart_final.assumptions.source.contains("synthetic"), "{}", chart_final.assumptions.source);
    with_charts.push(synthetic(EvReference::DecisionIncrementalVerified));
    let mut r = rig("injected", PreflopStore::from_sources(with_charts));
    let id = serve(&mut r, &state);
    let f = the_final(&finish(r), &id);
    assert!(f.assumptions.source.contains("synthetic_v2_100bb"), "{}", f.assumptions.source);
    assert_eq!(f.coverage, Coverage::Exact);
    assert!(f.actions.iter().all(|a| a.ev_bb.is_some()));
}

/// Ruling 17-pre: hero's node is looked up through the replay walk, which carries the exact source step it chose at
/// each translated edge, never by a chip-only query. At a three-chip big blind a 6-chip open (below 2.5 bb) clamps to
/// the source's 2500 edge, whose chip action `Raise{to: 8}` is also 2600's: the chip-only query cannot tell the two
/// edges apart and reports an unresolved missing node, while the engine answers from HJ's node after 2500, the edge
/// the open was mapped to.
#[test]
fn hero_lookups_go_through_the_replay_walk_not_a_chip_only_query() {
    let hj = |seed: usize| mem_node(Position::Hj, vec![PreflopStep::Fold, PreflopStep::Call], seed);
    let nodes: MemNodes = vec![
        (vec![], mem_node(Position::Utg, vec![PreflopStep::Fold, raise(2500), raise(2600)], 1)),
        (vec![(Position::Utg, raise(2500))], hj(2)),
        (vec![(Position::Utg, raise(2600))], hj(5)),
    ];
    let hj_after_2500 = nodes[1].1.clone();
    let state = act(&table_at(3, 100, HJ, "AhKd"), &[Action::Raise { to: 6 }]);
    let (_, class) = hero_class(&state);
    let store = mem_store(nodes);
    let out = replayed(&store, &state);
    assert_eq!(out.branches.len(), 1, "a clamp, not a split");
    assert_eq!(out.branches[0].translated, vec![(UTG, Action::Raise { to: 8 })]);
    let chip_only = core_replay::query_translated(&store, &state.config, &state, 1, &out.branches[0]);
    assert!(chip_only.node.is_none() && chip_only.key.contains("unresolved"), "{}", chip_only.key);
    let mut r = rig("walk_lookup", store);
    let id = serve(&mut r, &state);
    let f = the_final(&finish(r), &id);
    assert!(matches!(f.coverage, Coverage::Approximate { .. }), "{:?}", f.coverage);
    let row = &hj_after_2500.probs[class];
    let row_sum: f64 = row.iter().map(|p| f64::from(*p)).sum();
    assert_eq!(f.actions.iter().map(|a| a.action).collect::<Vec<_>>(), [Action::Fold, Action::Call]);
    for (advice, p) in f.actions.iter().zip(row) {
        close(f64::from(advice.frequency.unwrap()), f64::from(*p) / row_sum, 1e-6, "HJ's node after the 2500 edge");
    }
}

/// Decision 3 (plan-2 carry F-Q2, ruling 28-N1): the preflop path answers through the request's existing claim and
/// never re-arms anything. A request whose watchdog already delivered its `Final` (it waited past its fire) is served
/// afterwards: its `Fast` and its own `Final` are dropped, the watchdog's `Final` is the one logged, once, and only an
/// `Equity` follows the delivered `Final`.
#[test]
fn a_preflop_request_answers_through_its_claim_after_the_watchdog_delivered() {
    let mut r = rig("claimed", PreflopStore::from_sources(charts()));
    let ended = r.core.watchdog.ended_threads();
    let state = table(100, UTG, "AhAd");
    let req = admit(&r, &state);
    let fire = req.watch.as_ref().expect("a decision is armed at admission").deadlines.watchdog_fire_ms();
    r.clock.set_ms(fire);
    ended.wait_for(1);
    let id = serve_admitted(&mut r, req);
    let served = finish(r);
    assert_eq!(kinds(&served.events), ["Final", "Equity"], "the watchdog's Final, then only an Equity");
    let f = the_final(&served, &id);
    assert!(matches!(f.coverage, Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { .. }, .. }), "{:?}", f.coverage);
    assert_eq!(served.records.len(), 1, "logged once");
    assert!(served.records[0].final_violation);
}

/// A request whose decision is superseded before it is served emits nothing on the preflop path (no `Fast`, no
/// `Final`, no `Equity`) and logs nothing.
#[test]
fn a_superseded_preflop_request_emits_and_logs_nothing() {
    let mut r = rig("stale", PreflopStore::from_sources(charts()));
    let req = admit(&r, &table(100, UTG, "AhAd"));
    r.identity.lock().unwrap().mutate();
    serve_admitted(&mut r, req);
    let served = finish(r);
    assert!(served.events.is_empty(), "{:?}", kinds(&served.events));
    assert!(served.records.is_empty() && served.calls.is_empty());
}

// ---------------------------------------------------------------------------------------------
// The store the engine holds: loaded once, handed out by `Engine::preflop_store`.
// ---------------------------------------------------------------------------------------------

/// `Engine::preflop_store` hands out the store the core holds (loaded before the core is handed over), and a
/// recommendation through the public `Engine` reads it with no disk access: hero's RFI decision is the chart's.
#[test]
fn the_engine_hands_out_its_loaded_store_and_recommends_from_it() {
    use engine::Engine;
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let (worker, fake) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![]);
    let log_dir = TempDir::new("engine_log", false);
    let mut core = EngineCore::new(worker, clock.clone(), identity, DecisionLog::open(&log_dir));
    core.preflop = Arc::new(PreflopStore::from_sources(charts()));
    let calls: Arc<Mutex<Vec<EquityCall>>> = Arc::default();
    let mut e = Engine::with_core_and_seams(core, ServeSeams { equity: Some(stub_equity(calls.clone())), ..ServeSeams::default() });
    let ids: Vec<String> = e.preflop_store().bundles().iter().map(|b| b.bundle_info().bundle_id.clone()).collect();
    assert_eq!(ids, available_chart_depths().into_iter().map(|(_, id)| id).collect::<Vec<_>>());
    let (cfg, _) = engine::testing::cfg_1_2();
    e.set_config(cfg).unwrap();
    // The button at seat 5, so hero at seat 2 is UTG, first to act, holding AA.
    e.begin_hand(proto::BeginHand { button: Seat(5), hero: UTG, dealt: (0..6).map(Seat).collect(), stacks: vec![1000; 6], hero_cards: Some(core_model::parse_hand("AhAd").unwrap()) })
        .unwrap();
    let (sink, recorder) = RecordingSink::notifying(clock.clone(), None);
    let id = e.recommend(Box::new(sink)).unwrap();
    let got = recorder.wait_for(2);
    e.shutdown();
    let f = got.iter().find_map(|r| match &r.event { RecommendationEvent::Final(f) => Some(f.clone()), _ => None }).expect("the Final");
    assert_eq!(f.identity, id);
    assert!(reasons(&f).contains(&ApproxReason::ChartRounded) && f.assumptions.source.contains("pokercoaching_100"), "{:?}", f.coverage);
    assert!(!fake.lock().unwrap().sent.iter().any(|m| matches!(m, EngineMessage::Solve(_))));
}

/// Copies of the available chart pairs (`<id>.manifest.json` + `<id>.json`, the packaged layout) into a fresh
/// directory: the loader is never pointed at `fixtures/charts` itself, whose subdirectories a quarantine would rename.
fn staged_charts(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("store_{tag}"), true);
    for (_, id) in available_chart_depths() {
        for name in [format!("{id}.manifest.json"), format!("{id}.json")] {
            std::fs::copy(fixtures().join("charts").join(&name), dir.join(&name)).unwrap();
        }
    }
    dir
}

/// `load_store` reads the packaged sibling pairs of `paths.preflop` and the installed bundle directories, and
/// quarantines every failing one (spec 8.2, ruling 17-I1): a valid pair loads; a failing installed directory is
/// renamed `.bad` (the store's own quarantine) and a failing packaged pair is renamed `.bad` too, both files, at the
/// first free collision-safe name (here `.1.bad`, a stale `.bad` already being there), each with a banner, both listed
/// as quarantined; a second bundle under an id already loaded is skipped with a banner; every other source stays
/// active. A second load reads nothing quarantined. Synthetic fixtures are never read (nothing outside the directory is).
#[test]
fn the_store_loads_packaged_pairs_and_quarantines_every_failing_bundle() {
    use engine::preflop::load_store;
    let dir = staged_charts("mixed");
    let ids: Vec<String> = available_chart_depths().into_iter().map(|(_, id)| id).collect();
    let clean = load_store(&dir);
    assert_eq!(clean.store.bundles().iter().map(|b| b.bundle_info().bundle_id.clone()).collect::<Vec<_>>(), ids);
    assert!(clean.banners.is_empty() && clean.quarantined.is_empty(), "{:?}", clean.banners);
    assert!(clean.store.bundles().iter().all(|b| b.bundle_info().source == SourceKind::ChartTranscription), "only the packaged charts, never a synthetic bundle");
    // A broken installed bundle (a directory) and a broken packaged pair, plus a valid pair under an id already loaded.
    let broken = dir.join("broken_pd");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::copy(fixtures().join("preflop/synthetic_v2/manifest.json"), broken.join("manifest.json")).unwrap();
    std::fs::write(broken.join("nodes.json"), b"{}").unwrap();
    std::fs::copy(fixtures().join(format!("charts/{}.manifest.json", ids[0])), dir.join("broken_chart.manifest.json")).unwrap();
    std::fs::write(dir.join("broken_chart.json"), b"{}").unwrap();
    std::fs::copy(fixtures().join(format!("charts/{}.manifest.json", ids[0])), dir.join("zz_again.manifest.json")).unwrap();
    std::fs::copy(fixtures().join(format!("charts/{}.json", ids[0])), dir.join("zz_again.json")).unwrap();
    // A stale quarantine of an earlier broken_chart occupies the first name.
    std::fs::write(dir.join("broken_chart.json.bad"), b"stale").unwrap();
    let loaded = load_store(&dir);
    assert_eq!(loaded.store.bundles().iter().map(|b| b.bundle_info().bundle_id.clone()).collect::<Vec<_>>(), ids, "the valid sources stay active");
    assert_eq!(loaded.quarantined, vec!["broken_pd".to_string(), "broken_chart".to_string()]);
    assert!(!broken.exists() && dir.join("broken_pd.bad").is_dir(), "the installed bundle was renamed .bad");
    assert!(!dir.join("broken_chart.manifest.json").exists() && !dir.join("broken_chart.json").exists(), "the failing packaged pair is no longer under its loadable names");
    assert_eq!(std::fs::read(dir.join("broken_chart.json.1.bad")).unwrap(), b"{}", "renamed to the first free collision-safe name, never over the stale one");
    assert!(dir.join("broken_chart.manifest.json.1.bad").is_file() && std::fs::read(dir.join("broken_chart.json.bad")).unwrap() == b"stale");
    assert_eq!(loaded.banners.len(), 3, "{:?}", loaded.banners);
    assert!(loaded.banners[0].starts_with("preflop bundle broken_pd quarantined"), "{:?}", loaded.banners);
    assert!(loaded.banners[1].starts_with("packaged preflop bundle broken_chart quarantined as ") && loaded.banners[1].contains("broken_chart.manifest.json.1.bad"),
        "{:?}", loaded.banners);
    assert!(loaded.banners[2].contains("zz_again") && loaded.banners[2].contains(&ids[0]), "{:?}", loaded.banners);
    // The next start reads nothing quarantined: only the duplicate's banner remains.
    let again = load_store(&dir);
    assert_eq!(again.store.bundles().iter().map(|b| b.bundle_info().bundle_id.clone()).collect::<Vec<_>>(), ids);
    assert!(again.quarantined.is_empty(), "{:?}", again.quarantined);
    assert_eq!(again.banners.len(), 1, "{:?}", again.banners);
}

/// Ruling 17-I1: a failing packaged pair whose quarantine rename cannot be performed (its manifest is held open without
/// delete sharing, as a file in a location the engine may not write to cannot be renamed) is still excluded, the other
/// sources stay active, and the unsuccessful quarantine is reported as a clear, non-fatal banner; the pair stays under
/// its names and is listed as quarantined.
#[cfg(windows)]
#[test]
fn a_failing_packaged_pair_that_cannot_be_renamed_is_excluded_and_reported() {
    use engine::preflop::load_store;
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    let dir = staged_charts("unrenamable");
    let ids: Vec<String> = available_chart_depths().into_iter().map(|(_, id)| id).collect();
    std::fs::copy(fixtures().join(format!("charts/{}.manifest.json", ids[0])), dir.join("broken_chart.manifest.json")).unwrap();
    std::fs::write(dir.join("broken_chart.json"), b"{}").unwrap();
    // Readable by the loader, but no rename (delete access) while this handle lives.
    let held = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(dir.join("broken_chart.manifest.json")).unwrap();
    let loaded = load_store(&dir);
    drop(held);
    assert_eq!(loaded.store.bundles().iter().map(|b| b.bundle_info().bundle_id.clone()).collect::<Vec<_>>(), ids, "the valid sources stay active");
    assert_eq!(loaded.quarantined, vec!["broken_chart".to_string()]);
    assert_eq!(loaded.banners.len(), 1, "{:?}", loaded.banners);
    let banner = &loaded.banners[0];
    assert!(banner.starts_with("packaged preflop bundle broken_chart failed validation") && banner.contains("could not be quarantined") && banner.contains("excluded"),
        "{banner}");
    assert!(dir.join("broken_chart.manifest.json").is_file() && dir.join("broken_chart.json").is_file(), "nothing was renamed");
    assert!(!dir.join("broken_chart.manifest.json.bad").exists() && !dir.join("broken_chart.json.bad").exists(), "no half quarantine");
}

/// Ruling 17-N1: the packaged pass renames ordinary files only; a directory is `PreflopStore::open`'s candidate. A valid
/// installed bundle folder `foo.json/` beside a file `foo.manifest.json` (a failing pair: its nodes entry is no file)
/// stays a folder and keeps loading, while the pair's one file is quarantined; a valid installed bundle folder named like
/// a manifest (`bar.manifest.json/`) is not a packaged pair at all: no banner, nothing quarantined, nothing renamed.
#[test]
fn the_packaged_pass_never_touches_an_installed_bundle_directory() {
    use engine::preflop::load_store;
    let installed = |dir: &Path, name: &str| {
        std::fs::create_dir_all(dir.join(name)).unwrap();
        for file in ["manifest.json", "nodes.json"] {
            std::fs::copy(fixtures().join("preflop/synthetic_v2").join(file), dir.join(name).join(file)).unwrap();
        }
    };
    let ids = |loaded: &engine::preflop::LoadedStore| loaded.store.bundles().iter().map(|b| b.bundle_info().bundle_id.clone()).collect::<Vec<_>>();
    let charts: Vec<String> = available_chart_depths().into_iter().map(|(_, id)| id).collect();
    let with_synthetic: Vec<String> = std::iter::once("synthetic_v2_100bb".to_string()).chain(charts.iter().cloned()).collect();
    // A folder named like a nodes file, paired with a manifest file.
    let dir = staged_charts("installed_json");
    installed(&dir, "foo.json");
    std::fs::copy(fixtures().join(format!("charts/{}.manifest.json", charts[0])), dir.join("foo.manifest.json")).unwrap();
    let loaded = load_store(&dir);
    assert_eq!(ids(&loaded), with_synthetic, "the installed folder loads");
    assert!(dir.join("foo.json").is_dir() && dir.join("foo.json").join("nodes.json").is_file(), "the installed folder is untouched");
    assert!(dir.join("foo.manifest.json.bad").is_file() && !dir.join("foo.json.bad").exists(), "only the pair's file is quarantined");
    assert_eq!(loaded.quarantined, vec!["foo".to_string()]);
    assert_eq!(loaded.banners.len(), 1, "{:?}", loaded.banners);
    assert!(loaded.banners[0].starts_with("packaged preflop bundle foo quarantined as ") && !loaded.banners[0].contains("foo.json.bad"), "{:?}", loaded.banners);
    let again = load_store(&dir);
    assert_eq!((ids(&again), again.banners.clone(), again.quarantined.clone()), (with_synthetic, vec![], vec![]), "the next start still loads it, silently");
    // A folder named like a manifest.
    let dir = TempDir::new("store_installed_manifest", true);
    installed(&dir, "bar.manifest.json");
    for _ in 0..2 {
        let loaded = load_store(&dir);
        assert_eq!((ids(&loaded), loaded.banners.clone(), loaded.quarantined.clone()), (vec!["synthetic_v2_100bb".to_string()], vec![], vec![]));
        assert!(dir.join("bar.manifest.json").is_dir() && !dir.join("bar.manifest.json.bad").exists(), "never renamed");
    }
}

/// Ruling 17-N2: a pair's quarantine is all or nothing. When the manifest renames but the nodes file cannot (held open
/// without delete sharing), the manifest's rename is rolled back: both files keep their names, no `.bad` exists, the
/// banner reports the unsuccessful quarantine and the pair is excluded; the next start sees the pair and reports it
/// again, and once the file is released it is quarantined whole.
#[cfg(windows)]
#[test]
fn a_pair_whose_nodes_file_cannot_be_renamed_is_rolled_back_whole() {
    use engine::preflop::load_store;
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    let dir = staged_charts("half_renamed");
    let ids: Vec<String> = available_chart_depths().into_iter().map(|(_, id)| id).collect();
    std::fs::copy(fixtures().join(format!("charts/{}.manifest.json", ids[0])), dir.join("broken_chart.manifest.json")).unwrap();
    std::fs::write(dir.join("broken_chart.json"), b"{}").unwrap();
    let held = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(dir.join("broken_chart.json")).unwrap();
    for start in ["first", "second"] {
        let loaded = load_store(&dir);
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).filter(|n| n.starts_with("broken_chart")).collect();
        assert_eq!(loaded.store.bundles().iter().map(|b| b.bundle_info().bundle_id.clone()).collect::<Vec<_>>(), ids, "{start}: the valid sources stay active");
        assert_eq!(loaded.quarantined, vec!["broken_chart".to_string()], "{start}");
        assert_eq!(names, ["broken_chart.json", "broken_chart.manifest.json"], "{start}: both files keep their names, no .bad");
        assert_eq!(loaded.banners.len(), 1, "{start}: {:?}", loaded.banners);
        let banner = &loaded.banners[0];
        assert!(banner.starts_with("packaged preflop bundle broken_chart failed validation") && banner.contains("could not be quarantined")
            && banner.contains("broken_chart.json could not be renamed") && banner.contains("restored") && banner.contains("excluded"), "{start}: {banner}");
    }
    drop(held);
    let loaded = load_store(&dir);
    assert!(loaded.banners[0].starts_with("packaged preflop bundle broken_chart quarantined as "), "{:?}", loaded.banners);
    assert!(dir.join("broken_chart.manifest.json.bad").is_file() && dir.join("broken_chart.json.bad").is_file());
}

/// A preflop directory that is missing, or that holds no loadable source, leaves an empty store and says so in a banner:
/// every preflop decision then answers `MissingPreflopNode` (spec 5 step 6), never a construction error.
#[test]
fn a_missing_preflop_directory_is_a_banner_and_an_empty_store() {
    use engine::preflop::load_store;
    let dir = TempDir::new("store_absent", false);
    let loaded = load_store(&dir);
    assert!(loaded.store.bundles().is_empty() && loaded.quarantined.is_empty());
    assert!(loaded.banners.iter().any(|b| b.contains("no preflop bundle is loaded")), "{:?}", loaded.banners);
    let mut r = rig("empty_store", loaded.store);
    let id = serve(&mut r, &table(100, UTG, "AhAd"));
    let f = the_final(&finish(r), &id);
    // The key is the lookup's own statement of the gap, never an invented one, and it is in the assumptions too (spec
    // 12, ruling 17-I2).
    assert_eq!(f.coverage, Coverage::Unsupported { reason: UnsupportedReason::MissingPreflopNode { key: "no bundle".into() }, partial: vec![] });
    assert!(f.assumptions.notes.contains(&"missing preflop node: no bundle".to_string()), "{:?}", f.assumptions.notes);
}
