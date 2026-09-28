//! Plan 3 Task 17: the engine's chart and EV-bearing preflop decision path (spec 5 steps 5-6, spec 6's preflop rows,
//! spec 8.3 and 8.4, spec 4.4's one headline rule), and the preflop store the engine loads once (spec 8.2).
//!
//! # One request
//!
//! `serve_preflop` answers a preflop decision point on `engine-main`, from `serve_request`'s
//! `Classification::Preflop` arm, which sits after the degraded-engine check (a degraded engine answers every decision
//! with the version mismatch first). In order:
//!
//! 1. replay every observed preflop action over the loaded store (`core_replay::replay_decision`, spec 9): the shared
//!    history branches, every seat's public range, the replay's reasons, and hero's current node as the replay walk
//!    looked it up in every live branch -- under the branch's translated history with the exact source step it carried
//!    at each translated edge (ruling 17-pre), never a standalone chip-only query;
//! 2. emit the `Fast` (legal intervals, the coverage so far, equity pending) through `serve::emit` with the request's
//!    once-only `Final` flag, so it is dropped once the watchdog delivered;
//! 3. map hero's node in each branch onto the live legal menu (spec 8.4's legality after mapping: `destination_map` once
//!    per node, `legalize_row` for every combo row, `BranchNode::created` from `legalize_row`'s `MovedProbability`) and
//!    assemble hero's decision over the branches (`core_preflop::mix_nodes`, handed the raw legalized rows: it normalizes
//!    each admitted source row once, at the mapped-node boundary);
//! 4. deliver the `Final` through the request's own claim (`serve::settle`, shared with the watchdog armed at admission):
//!    nothing is armed, disarmed or re-armed here, no engine lock is held during a sink callback, and nothing is
//!    registered in the snapshot store (a preflop decision has no street solution);
//! 5. start the request's `fast-path` equity (spec 6: every preflop row shows equity) over the replayed public ranges:
//!    hero's public range against every other seat still in the hand, delivered as an `Equity` event after the `Final`.
//!
//! No `solve` is ever sent on this path, and no disk is read: the store was loaded before the core was handed over.
//!
//! # The `Final` (spec 6's preflop rows)
//!
//! Known masses (a node present in every or in some positive-posterior branches) go through Plan 2's
//! `assemble::final_from_known_mass` (plan-2 Task 26 carry), whose one headline rule (`assemble::headline`) takes the
//! source's wording from [`headline_source`]: "highest EV" when every action has an EV, the chart and the
//! unverified-reference frequency wordings, none for a solved source whose EVs are incomplete, and none of any kind
//! while `unresolved_mass > 0`. `MissingPreflopNode` (hero has a node in no positive-posterior branch) is
//! `assemble::unsupported`: equity only, every inherited reason in `partial`. `HeroComboOutOfSupport` keeps the
//! range-level mix as its only strategy output. A replay the store cannot answer at all (`ReplayOutput::unsupported`)
//! is `Unsupported` with that reason. Reasons accumulate, each once by value (`assemble::accumulate`): the replay's
//! (translations, stops, earlier actors' mappings, the cap residual), then hero's own lookup's mappings, then the
//! assembly's. Charts carry `ChartRounded` and no EV (their EV absence says nothing about a later postflop EV: charts are
//! incoming-range provenance there).
//!
//! # Two residual disclosures (Task 15 carry, rulings 15-I1 and 15-Q5)
//!
//! The replay boundary discloses a cap residual as `BranchResidual{seat: hero, cause: "cap"}`, its share of the branch
//! weights; the assembly adds `BranchResidual{seat: hero, cause: "missing node <key>"}`, hero's posterior share with no
//! strategy, whenever `unresolved_mass > 0`. They are different disclosures and both are kept, deduplicated by value
//! only, whenever a node is actually missing (a stopped branch, or a live branch without hero's node): the cap share and
//! hero's unresolved share name different things. When the residual is the only positive-posterior branch without a
//! node, no node is missing, and the assembly's wording would name a key whose node is present: that one reason is then
//! not added, and a note says the unresolved share is the cap residual's. Nothing is lost: the cap reason stays, and
//! hero's exact share stays on the `Final` as `unresolved_mass` and its "x% of the posterior has no strategy" note.
//!
//! # The store (spec 8.2; Plan 5 stages the packaged charts into `Paths::preflop`)
//!
//! [`load_store`] is what `Engine::new` runs, once, on `Paths::preflop`: installed bundle directories
//! (`<bundle_id>/manifest.json` + `nodes.json`) through `PreflopStore::open`, whose quarantine renames a failing one
//! `.bad` with a banner, then the packaged sibling pairs (`<name>.manifest.json` + `<name>.json`) through the same
//! validated loader (`core_preflop::load_bundle`), a failing pair quarantined the same way (both files renamed `.bad`
//! at collision-safe names, with a banner; ruling 17-I1), or, when a rename cannot be performed, excluded with a banner
//! reporting the unsuccessful quarantine. Every other source stays active; synthetic fixtures are never loaded (nothing
//! outside the directory is read).

use crate::assemble::{self, AssemblyCtx, HeadlineSource, KnownMass};
use crate::core::EngineCore;
use crate::equity::pending_summary;
use crate::serve::{deadline_fallback, elapsed_ms, emit, settle, spawn_equity, Claim, Hooks, LiveRequest};
use crate::snapshots::StreetSnapshot;
use core_preflop::{
    destination_map, legalize_row, load_bundle, mix_nodes, BranchNode, BundleInfo, EvReference, ExpandedNode, MixedNode, PreflopNode, PreflopNodeKey,
    PreflopSource, PreflopStore, SourceKind,
};
use core_ranges::{mass, range_to_string};
use core_replay::{posterior, replay_decision, DecisionLookup, HistoryBranch, ReplayInput, ReplayOutput};
use proto::{
    Action, ApproxReason, Assumptions, Coverage, HandState, LegalAction, Range1326, Recommendation, RecommendationEvent, Seat, Street, Unavailable,
    UnsupportedReason, COMBOS,
};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

/// Spec 4.4 rule 2's wording for a preflop source: a chart is a chart (`"highest-frequency chart action"`), PokerData
/// with an unverified EV reference is the source-accuracy wording, and PokerData with a verified reference is a solved
/// source (a complete EV menu is "highest EV"; an incomplete one has no source wording). The label itself is
/// `assemble::headline`'s, the one rule (no second headline function).
pub fn headline_source(source: SourceKind, ev_reference: EvReference) -> HeadlineSource {
    match source {
        SourceKind::ChartTranscription => HeadlineSource::Chart,
        SourceKind::PokerDataJson if ev_reference == EvReference::Unverified => HeadlineSource::PokerDataUnverified,
        SourceKind::PokerDataJson => HeadlineSource::Solved,
    }
}

/// A lock that survives a panic elsewhere (see `serve`'s `lock`): the snapshot store and the request's fallback slot stay
/// consistent at every point a panic could interrupt them.
fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Spec 5 steps 5-6 for one preflop decision point (see the module doc): the replay, the `Fast`, the `Final` through
/// `claim`, then the request's equity. `ctx` is the request's assembly context (its legal intervals, hero's combo, the
/// big blind) and `assumptions` its starting assumptions; `equity_cancel` is the request's equity token (ruling 28-I4).
/// Called by `serve` only, after the degraded-engine check; every lock taken here is released before any emission.
pub(crate) fn serve_preflop(core: &mut EngineCore, req: &LiveRequest, hooks: &Hooks, claim: &Claim<'_>, equity_cancel: &Arc<AtomicBool>, ctx: &AssemblyCtx,
    assumptions: Assumptions) {
    let store = core.preflop.clone();
    // The replay's precondition (P3.T14): the snapshots of this decision's hand, config and model revision (the
    // identity carries the hand's config revision, ruling F-I4). A preflop decision walks no postflop street.
    let snapshots = lock(&core.snapshots).for_identity(&req.identity);
    let replayed = Replayed::run(&req.state, &store, &snapshots);
    let opponents = opponents(&replayed.output, &req.state);
    let mut ctx = ctx.clone();
    ctx.equity = pending_summary(&opponents.iter().map(|(seat, _)| *seat).collect::<Vec<_>>());
    let mut assumptions = assumptions;
    replayed.provenance(&mut assumptions, &req.state);
    // Ruling 28-I6: a watchdog `Final` from here on keeps the replay's reasons and the provenance.
    *lock(&claim.watch.fallback) = deadline_fallback(&ctx, &replayed.inherited, &assumptions);
    let coverage = assemble::accumulate(Coverage::Exact, replayed.inherited.clone());
    emit(core, req, Some(&*claim.watch.delivered), RecommendationEvent::Fast(assemble::fast(&ctx, coverage, assumptions.clone())));
    if let Some(after_fast) = &hooks.after_fast {
        after_fast();
    }
    assumptions.elapsed_ms = elapsed_ms(req.t0_ms, core.clock.now_ms());
    let rec = replayed.decide(&ctx, &req.state, assumptions);
    settle(core, req, hooks, claim, Street::Preflop, rec);
    // Spec 6: equity on every preflop row, `Final` or not, after the `Final` (the `fast-path` thread checks the identity
    // itself: a request no longer active computes nothing).
    if let Some(hero_public) = replayed.output.ranges.get(usize::from(req.state.hero.0)).cloned().flatten() {
        spawn_equity(core, req, hero_public, opponents, vec![], hooks.equity.clone(), equity_cancel.clone());
    }
}

/// Spec 5 step 6's `Final` for a preflop decision point, standalone: replays `state` over `store` and `snapshots` (the
/// decision's own, see `SnapshotStore::for_identity`) and assembles hero's decision on `ctx` (see the module doc).
/// `assumptions` are the request's starting assumptions; the provenance of the participating bundles is added to them.
pub fn preflop_final(ctx: &AssemblyCtx, state: &HandState, store: &PreflopStore, snapshots: &[StreetSnapshot], assumptions: Assumptions) -> Recommendation {
    let replayed = Replayed::run(state, store, snapshots);
    let mut assumptions = assumptions;
    replayed.provenance(&mut assumptions, state);
    replayed.decide(ctx, state, assumptions)
}

/// One replay of a preflop decision and what the `Fast` and the `Final` read from it.
struct Replayed {
    output: ReplayOutput,
    /// Hero's current node in every live branch, as the replay walk looked it up (ruling 17-pre).
    lookups: Vec<DecisionLookup>,
    /// The replay's reasons, then hero's own lookups' (their mapping reasons), each once by value.
    inherited: Vec<ApproxReason>,
    /// Hero's own lookups' notes, each once.
    notes: Vec<String>,
    /// The bundle ranked first at each observed preflop prefix, hero's decision's included, each once in prefix order:
    /// the bundles that participated (spec 8.3's ranking reads the observed prefix only, so every branch's lookup at one
    /// prefix is answered by the same bundle).
    bundles: Vec<BundleInfo>,
}

impl Replayed {
    fn run(state: &HandState, store: &PreflopStore, snapshots: &[StreetSnapshot]) -> Self {
        let (output, lookups) = replay_decision(ReplayInput { cfg: &state.config, state, store, snapshots });
        let mut inherited = Vec::new();
        for r in output.reasons.iter().chain(lookups.iter().flat_map(|l| l.answer.reasons.iter())) {
            push_unique(&mut inherited, r.clone());
        }
        let mut notes = Vec::new();
        for n in lookups.iter().flat_map(|l| l.answer.notes.iter()) {
            push_unique(&mut notes, n.clone());
        }
        // Hero's lookups at one prefix rank the bundles alike (spec 8.3 reads the observed prefix only).
        let decision_bundle = lookups.iter().find_map(|l| l.answer.bundle.clone());
        for l in &lookups {
            if let (Some(a), Some(b)) = (&l.answer.bundle, &decision_bundle) {
                assert!(a.bundle_id == b.bundle_id, "preflop: branch {}'s lookup at hero's decision is answered by {}, another branch's by {}", l.branch_id,
                    a.bundle_id, b.bundle_id);
            }
        }
        let prefix = state.actions.iter().take_while(|a| a.street == Street::Preflop).count();
        let mut bundles: Vec<BundleInfo> = Vec::new();
        for info in (0..prefix).filter_map(|i| store.query(&state.config, state, i).bundle).chain(decision_bundle) {
            if !bundles.iter().any(|b| b.bundle_id == info.bundle_id) {
                bundles.push(info);
            }
        }
        Replayed { output, lookups, inherited, notes, bundles }
    }

    /// Spec 4.4's assumptions of a preflop decision: the ranges used (every seat still in the hand, public, with its
    /// mass), the participating bundles and their accuracy, the 169-class granularity, the translations and mappings
    /// among the inherited reasons, and the lookups' notes.
    fn provenance(&self, assumptions: &mut Assumptions, state: &HandState) {
        assumptions.ranges_used =
            in_hand(state).into_iter().filter_map(|seat| self.output.ranges[usize::from(seat.0)].as_ref().map(|r| (seat, range_to_string(r), mass(r)))).collect();
        assumptions.source = if self.bundles.is_empty() {
            "preflop store: no bundle answered".into()
        } else {
            let named: Vec<String> = self.bundles.iter().map(|b| format!("{} ({:?}, {} bb)", b.bundle_id, b.source, b.depth_bb)).collect();
            format!("preflop store: {}", named.join(", "))
        };
        // One stated accuracy when every participating bundle states the same, else the weakest claim.
        let mut accuracies: Vec<&str> = self.bundles.iter().map(|b| b.accuracy.as_str()).collect();
        accuracies.sort_unstable();
        accuracies.dedup();
        assumptions.source_accuracy = match accuracies.as_slice() {
            [one] => (*one).to_string(),
            _ => "unverified".into(),
        };
        assumptions.source_granularity = "169-class".into();
        assumptions.translations = self.inherited.iter().filter(|r| matches!(r, ApproxReason::BetTranslation { .. })).cloned().collect();
        assumptions.mappings = self.inherited.iter().filter(|r| is_mapping(r)).cloned().collect();
        for n in &self.notes {
            push_unique(&mut assumptions.notes, n.clone());
        }
    }

    /// Hero's decision (spec 6's preflop rows; see the module doc).
    fn decide(&self, ctx: &AssemblyCtx, state: &HandState, mut assumptions: Assumptions) -> Recommendation {
        let inherited = self.inherited.clone();
        if let Some(reason) = self.output.unsupported.clone() {
            return assemble::unsupported(ctx, reason, inherited, assumptions);
        }
        let Some(hero_combo) = ctx.hero_combo.map(usize::from) else {
            return assemble::unsupported(ctx, UnsupportedReason::UnsupportedHistory { reason: "hero cards unknown".into() }, inherited, assumptions);
        };
        let branches = &self.output.branches;
        let mut nodes: Vec<BranchNode> = Vec::new();
        let mut source: Option<(SourceKind, EvReference)> = None;
        for b in branches.iter().filter(|b| !b.residual) {
            // Spec 9.3: a branch stopped on the preflop street has no node for any seat; its key is where it stopped.
            if let Some(cause) = &b.stopped {
                nodes.push(BranchNode { branch_id: b.id, key: cause.strip_prefix("missing node ").unwrap_or(cause).to_string(), ..Default::default() });
                continue;
            }
            let lookup = self.lookups.iter().find(|l| l.branch_id == b.id).unwrap_or_else(|| panic!("preflop: live branch {} has no lookup from the replay walk", b.id));
            let answer = &lookup.answer;
            let Some(expanded) = &answer.expanded else {
                match &answer.unsupported {
                    // A coverage gap (spec 9.3): the key the lookup used, or the lookup's own statement of the gap
                    // ("no bundle", "no acquired depth") when it built none.
                    Some(UnsupportedReason::MissingPreflopNode { key }) => {
                        let key = if answer.key.is_empty() { key.clone() } else { answer.key.clone() };
                        nodes.push(BranchNode { branch_id: b.id, key, ..Default::default() });
                        continue;
                    }
                    // A format, history or config the store cannot map at all makes the whole decision unsupported,
                    // exactly as the walk's own transactions mark the replay (`core_replay`'s missing-node stop).
                    Some(other) => return assemble::unsupported(ctx, other.clone(), inherited, assumptions),
                    None => panic!("preflop: branch {}'s lookup at hero's decision has no node and no reason: {answer:?}", b.id),
                }
            };
            match legalize(expanded, &ctx.legal, hero_combo, &mut assumptions.notes) {
                Ok((node, created)) => {
                    source.get_or_insert((expanded.source, expanded.ev_reference));
                    nodes.push(BranchNode { branch_id: b.id, node: Some(node), key: lookup.answer.key.clone(), created });
                }
                // Spec 8.4: a node with no legal destination for some source action is unsupported for every row alike.
                Err(reason) => return assemble::unsupported(ctx, reason, inherited, assumptions),
            }
        }
        let MixedNode { actions, unresolved_mass, range_mix, mut reasons, notes, unsupported } = mix_nodes(branches, &nodes, state.hero, hero_combo, ctx.bb_chips);
        if residual_alone(branches, &nodes, state.hero, hero_combo) && drop_missing_node_residual(&mut reasons, state.hero) {
            assert!(inherited.iter().any(|r| is_hero_residual(r, state.hero, |cause| cause == "cap")),
                "preflop: a residual branch holds hero's unresolved share, but the replay disclosed no cap residual: {inherited:?}");
            assumptions.notes.push("the unresolved share is the cap residual's (BranchResidual cause \"cap\"): no node is missing".into());
        }
        match unsupported {
            None => {
                let (kind, reference) = source.expect("preflop: advice was assembled, so a node answered in some branch");
                let known = KnownMass { actions, unresolved_mass, range_mix, reasons, notes };
                assemble::final_from_known_mass(ctx, known, assemble::accumulate(Coverage::Exact, inherited), headline_source(kind, reference), assumptions)
            }
            // Spec 6: equity and the range-level mix, which is the only strategy output; no per-combo number for hero.
            Some(UnsupportedReason::HeroComboOutOfSupport) => {
                for n in notes {
                    push_unique(&mut assumptions.notes, n);
                }
                let mut rec = assemble::unsupported(ctx, UnsupportedReason::HeroComboOutOfSupport, [inherited, reasons].concat(), assumptions);
                rec.actions = actions;
                rec.range_mix = range_mix;
                rec.unresolved_mass = unresolved_mass;
                rec
            }
            // `MissingPreflopNode`: equity only, every inherited reason kept in `partial`, and the key the assembly
            // selected in the assumptions too (spec 12, ruling 17-I2), exactly as `Coverage` names it.
            Some(reason) => {
                for n in notes {
                    push_unique(&mut assumptions.notes, n);
                }
                if let UnsupportedReason::MissingPreflopNode { key } = &reason {
                    push_unique(&mut assumptions.notes, missing_node_note(key));
                }
                assemble::unsupported(ctx, reason, [inherited, reasons].concat(), assumptions)
            }
        }
    }
}

/// Spec 12's `Unsupported{MissingPreflopNode}` "with the key in assumptions" (ruling 17-I2): the note carrying the exact
/// key the `Final`'s coverage names.
fn missing_node_note(key: &str) -> String {
    format!("missing preflop node: {key}")
}

/// The seats still in the hand (dealt, not folded), by seat id.
fn in_hand(state: &HandState) -> Vec<Seat> {
    let mut seats: Vec<Seat> = state.dealt.iter().copied().filter(|s| !state.derived.folded[usize::from(s.0)]).collect();
    seats.sort_by_key(|s| s.0);
    seats
}

/// Every other seat still in the hand with its replayed public range: the populations hero's equity is shown against
/// (spec 4.4, spec 6). Public ranges only; hero's cards enter no range here.
fn opponents(output: &ReplayOutput, state: &HandState) -> Vec<(Seat, Range1326)> {
    in_hand(state).into_iter().filter(|s| *s != state.hero).filter_map(|s| output.ranges[usize::from(s.0)].clone().map(|r| (s, r))).collect()
}

/// Spec 4.4's mapping reasons (the lookup's own, spec 8.3), as distinct from translations and approximations.
fn is_mapping(r: &ApproxReason) -> bool {
    matches!(
        r,
        ApproxReason::DepthBucket { .. }
            | ApproxReason::AsymmetricStacks { .. }
            | ApproxReason::RakeProfileMapped { .. }
            | ApproxReason::StraddleMapped { .. }
            | ApproxReason::ShortHandedMapped { .. }
    )
}

/// Hero's node in one branch mapped onto the live legal menu (spec 8.4's legality after mapping): the destination map
/// once for the node (`destination_map`), then every combo's row walked through it (`legalize_row`), unnormalized
/// (`mix_nodes` normalizes each admitted source row once). Returns the mapped node and its legality-created
/// destinations, each with the original source action whose probability was moved there first, as `legalize_row`
/// marks them (`MovedProbability{from}`, the same for every row). Hero's row's notes (the moves and collisions) are
/// added to `notes`.
///
/// # Panics
/// Always, if `legalize_row` reports a per-row unsupported verdict (it reports none: the whole-node verdict is
/// `destination_map`'s `Err`), or through `legalize_row`'s own checks.
fn legalize(expanded: &ExpandedNode, legal: &[LegalAction], hero_combo: usize, notes: &mut Vec<String>) -> Result<(ExpandedNode, Vec<(Action, Action)>), UnsupportedReason> {
    let (menu, map) = destination_map(&expanded.actions, legal)?;
    let mut probs = Vec::with_capacity(COMBOS);
    let mut evs = Vec::with_capacity(COMBOS);
    let mut created = Vec::new();
    for c in 0..COMBOS {
        let row = legalize_row(&menu, &map, &expanded.probs[c], &expanded.ev_chips[c], &mut Vec::new());
        assert!(row.unsupported.is_none(), "preflop: legalize_row reported {:?} for combo {c}; a node-wide verdict is destination_map's", row.unsupported);
        if c == hero_combo {
            for n in &row.notes {
                push_unique(notes, n.clone());
            }
            created = row
                .actions
                .iter()
                .filter_map(|a| match &a.unavailable { Some(Unavailable::MovedProbability { from }) => Some((a.action, *from)), _ => None })
                .collect();
        }
        probs.push(row.actions.iter().map(|a| a.probability).collect());
        evs.push(row.actions.iter().map(|a| a.ev_chips).collect());
    }
    let node = ExpandedNode {
        actor: expanded.actor,
        actions: menu,
        probs,
        ev_chips: evs,
        available: expanded.available.clone(),
        ev_reference: expanded.ev_reference,
        source: expanded.source,
    };
    Ok((node, created))
}

/// Whether hero's combo has positive posterior on some branch without a node, and every such branch is the residual:
/// the unresolved share is the cap residual's alone, and no node is missing (see the module doc).
fn residual_alone(branches: &[HistoryBranch], nodes: &[BranchNode], hero: Seat, hero_combo: usize) -> bool {
    let pi = posterior(branches, hero, hero_combo);
    let has_node = |b: &HistoryBranch| !b.residual && b.stopped.is_none() && nodes.iter().any(|n| n.branch_id == b.id && n.node.is_some());
    let without: Vec<&HistoryBranch> = branches.iter().zip(&pi).filter(|(b, p)| **p > 0.0 && !has_node(b)).map(|(b, _)| b).collect();
    !without.is_empty() && without.iter().all(|b| b.residual)
}

/// `reasons` without the assembly's hero "missing node" residual; whether it held one.
///
/// # Panics
/// Always, if it held more than one (the assembly adds at most one).
fn drop_missing_node_residual(reasons: &mut Vec<ApproxReason>, hero: Seat) -> bool {
    let before = reasons.len();
    reasons.retain(|r| !is_hero_residual(r, hero, |cause| cause.starts_with("missing node ")));
    assert!(before - reasons.len() <= 1, "preflop: the assembly disclosed {} missing-node residuals", before - reasons.len());
    reasons.len() < before
}

/// Whether `r` is hero's `BranchResidual` whose cause satisfies `cause`.
fn is_hero_residual(r: &ApproxReason, hero: Seat, cause: impl Fn(&str) -> bool) -> bool {
    matches!(r, ApproxReason::BranchResidual { seat, cause: c, .. } if *seat == hero && cause(c))
}

/// Keeps first occurrences only (by value).
fn push_unique<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}

// ---------------------------------------------------------------------------------------------
// The store the engine loads once (spec 8.2).
// ---------------------------------------------------------------------------------------------

/// What [`load_store`] loaded from a preflop directory: the store, one startup banner per rejected or skipped source (and
/// one when nothing loaded), and the names of the sources that failed validation and were excluded: quarantined
/// (renamed `.bad`: an installed directory, or both files of a packaged pair), or, where the rename could not be
/// performed, excluded in place with a banner saying so.
pub struct LoadedStore {
    pub store: PreflopStore,
    pub banners: Vec<String>,
    pub quarantined: Vec<String>,
}

/// The acquisition record (plan 3 Task 4) that sits beside the packaged chart pairs in `fixtures/charts`: not a bundle.
const ACQUISITION_RECORD: &str = "sources.manifest.json";

/// Loads the preflop store from `dir` (`Paths::preflop`), once, at the engine's construction (spec 5 step 1, spec 8.2):
///
/// 1. installed bundles, `<bundle_id>/manifest.json` + `nodes.json` (PokerData's layout), through
///    `PreflopStore::open`: each validated on its own bytes, link safety first, a failing one renamed `.bad` with a
///    banner;
/// 2. packaged bundles, the sibling pairs `<name>.manifest.json` + `<name>.json` (the chart layout Plan 5 stages here),
///    in name order, through the same validated loader (`core_preflop::load_bundle`); a pair whose file is a link is
///    refused unread. A failing pair is quarantined like a failing directory (spec 8.2, ruling 17-I1): both of its
///    entries renamed `.bad` at the first free collision-safe names, with a banner (`quarantine_pair`); when a rename
///    cannot be performed (an unwritable location, a file held open), the pair is still excluded and a banner reports
///    the unsuccessful quarantine, non-fatally. A pair whose `bundle_id` is already loaded is skipped with a banner (the
///    first one loaded wins, installed before packaged); the acquisition record `sources.manifest.json` is not a bundle
///    and is skipped.
///
/// Every other source stays active. A missing or unreadable directory, or one with nothing loadable, leaves an empty
/// store and a banner: every preflop decision then answers `MissingPreflopNode`, never a construction error. Nothing
/// outside `dir` is read, so synthetic fixtures are never loaded in production.
pub fn load_store(dir: &Path) -> LoadedStore {
    // The installed candidates, named before `open` renames a failing one.
    let candidates = installed_candidates(dir);
    let (installed, mut banners) = PreflopStore::open(dir);
    let mut quarantined: Vec<String> = candidates.into_iter().filter(|name| banners.iter().any(|b| b.starts_with(&format!("preflop bundle {name} ")))).collect();
    let installed = Arc::new(installed);
    let mut sources: Vec<Box<dyn PreflopSource>> =
        (0..installed.bundles().len()).map(|index| Box::new(Installed { store: installed.clone(), index }) as Box<dyn PreflopSource>).collect();
    for name in file_names(dir) {
        let Some(stem) = name.strip_suffix(".manifest.json") else { continue };
        if name == ACQUISITION_RECORD {
            continue;
        }
        match packaged_pair(dir, &name, stem) {
            Ok(source) => {
                let id = source.bundle_info().bundle_id.clone();
                if sources.iter().any(|s| s.bundle_info().bundle_id == id) {
                    banners.push(format!("packaged preflop bundle {stem} skipped: bundle_id {id} is already loaded"));
                } else {
                    sources.push(source);
                }
            }
            Err(why) => {
                banners.push(match quarantine_pair(dir, &name, stem) {
                    Ok(renamed) => format!("packaged preflop bundle {stem} quarantined as {renamed}: {why}"),
                    Err(failed) => format!(
                        "packaged preflop bundle {stem} failed validation ({why}) and could not be quarantined ({failed}); it is excluded until it is fixed or removed"
                    ),
                });
                quarantined.push(stem.to_string());
            }
        }
    }
    let store = PreflopStore::from_sources(sources);
    if store.bundles().is_empty() {
        banners.push(format!("no preflop bundle is loaded from {}: every preflop decision answers MissingPreflopNode until one is installed", dir.display()));
    }
    LoadedStore { store, banners, quarantined }
}

/// The immediate children of `dir` that `PreflopStore::open` takes as bundle candidates (a directory or a link, not
/// already named `.bad`); none when `dir` cannot be read (`open` reports that).
fn installed_candidates(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_dir() || m.file_type().is_symlink()))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| !name.ends_with(".bad"))
        .collect()
}

/// The names of `dir`'s immediate children, sorted; none when `dir` cannot be read (`open` reports that).
fn file_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut names: Vec<String> = entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    names
}

/// One packaged pair, `<stem>.manifest.json` + `<stem>.json` in `dir`, through the validated loader; both must be
/// ordinary files (a link is refused before anything is opened).
fn packaged_pair(dir: &Path, manifest_name: &str, stem: &str) -> Result<Box<dyn PreflopSource>, String> {
    let (manifest, nodes) = (dir.join(manifest_name), dir.join(format!("{stem}.json")));
    for path in [&manifest, &nodes] {
        let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("{} is not an ordinary file", path.display()));
        }
    }
    load_bundle(&manifest, &nodes).map_err(|e| e.to_string())
}

/// Whether `path` names an existing entry, a link included, even a dangling one (never followed): a quarantine name in
/// use (as `core_preflop`'s store judges its own quarantine names).
fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Spec 8.2's quarantine of a failing packaged pair (ruling 17-I1), the packaged twin of `PreflopStore::open`'s: each of
/// its entries present in `dir` (the manifest `manifest_name`, then the nodes file `<stem>.json` when there is one) is
/// renamed within `dir` to `<entry>.bad`, or, when either name is taken, to `<entry>.<n>.bad` with the smallest `n >= 1`
/// free for both, so a stale quarantine is never overwritten and the pair keeps one suffix. A link is renamed as the link
/// entry itself, never its target, and every source and target is an immediate child of `dir`. Returns the new paths;
/// `Err` names the rename that failed (an unwritable location, a file held open) and any entry already renamed, so the
/// caller reports the unsuccessful quarantine and still excludes the pair. The manifest goes first: while it keeps its
/// name the pair is not half quarantined.
fn quarantine_pair(dir: &Path, manifest_name: &str, stem: &str) -> Result<String, String> {
    let nodes_name = format!("{stem}.json");
    let entries: Vec<&str> = [manifest_name, nodes_name.as_str()].into_iter().filter(|name| occupied(&dir.join(name))).collect();
    let suffix = (0u32..)
        .map(|n| if n == 0 { ".bad".to_string() } else { format!(".{n}.bad") })
        .find(|suffix| entries.iter().all(|name| !occupied(&dir.join(format!("{name}{suffix}")))))
        .expect("a free quarantine name exists");
    let mut renamed: Vec<String> = Vec::new();
    for name in entries {
        let (from, to) = (dir.join(name), dir.join(format!("{name}{suffix}")));
        let done = if renamed.is_empty() { String::new() } else { format!("; already renamed: {}", renamed.join(", ")) };
        if from.parent() != Some(dir) || to.parent() != Some(dir) {
            return Err(format!("{} would leave the preflop directory{done}", from.display()));
        }
        std::fs::rename(&from, &to).map_err(|e| format!("{} could not be renamed to {}: {e}{done}", from.display(), to.display()))?;
        renamed.push(to.display().to_string());
    }
    Ok(renamed.join(" and "))
}

/// A bundle `PreflopStore::open` admitted, kept in the engine's one store beside the packaged ones: it answers exactly
/// as the admitted bundle does (a chart's lookup still clears every EV).
struct Installed {
    store: Arc<PreflopStore>,
    index: usize,
}

impl Installed {
    fn bundle(&self) -> &dyn PreflopSource {
        self.store.bundles()[self.index].as_ref()
    }
}

impl PreflopSource for Installed {
    fn bundle_info(&self) -> &BundleInfo {
        self.bundle().bundle_info()
    }
    fn lookup(&self, key: &PreflopNodeKey) -> Option<PreflopNode> {
        self.bundle().lookup(key)
    }
    fn nodes_have_no_ev(&self) -> bool {
        self.bundle().nodes_have_no_ev()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{Card, DecisionIdentity};

    /// Hero's own lookup at the decision: a coverage gap with no key built keeps the lookup's own statement of it
    /// ("no bundle"), and an answer unsupported for any other reason (a format, history or config the store cannot map)
    /// makes the decision `Unsupported` with that reason, as the walk's transactions mark the replay, never
    /// `MissingPreflopNode`. No committed fixture reaches the second answer at hero's own lookup, so it is set here.
    #[test]
    fn a_non_gap_answer_at_heros_lookup_is_the_decisions_reason() {
        let cards = [Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()];
        let state = crate::testing::hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(5), Seat(2), Some(cards));
        let ctx = AssemblyCtx {
            identity: DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 },
            legal: state.derived.legal.clone(),
            hero_combo: Some(proto::combo_index(cards[0], cards[1])),
            bb_chips: state.config.bb_chips,
            equity: pending_summary(&[]),
        };
        let mut replayed = Replayed::run(&state, &PreflopStore::from_sources(vec![]), &[]);
        assert_eq!(replayed.lookups.len(), 1, "one live branch, hero first to act");
        let gap = replayed.decide(&ctx, &state, assemble::empty_assumptions(""));
        assert_eq!(gap.coverage, Coverage::Unsupported { reason: UnsupportedReason::MissingPreflopNode { key: "no bundle".into() }, partial: vec![] });
        let reason = UnsupportedReason::UnsupportedHistory { reason: "a raise with no representation in source units".into() };
        replayed.lookups[0].answer.unsupported = Some(reason.clone());
        let rec = replayed.decide(&ctx, &state, assemble::empty_assumptions(""));
        assert_eq!(rec.coverage, Coverage::Unsupported { reason, partial: vec![] });
        assert!(rec.actions.iter().all(|a| a.frequency.is_none() && a.unavailable == Some(Unavailable::NotEvaluated)), "{:?}", rec.actions);
    }
}
