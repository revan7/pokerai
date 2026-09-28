//! The postflop walk: each completed street replayed through the observed actions of the one
//! snapshot selected for it (spec sections 8.4, 9.2 and 9.3; plan 3 Task 15).
//!
//! # The street root
//!
//! [`crate::replay`] blocks the street's root board on every seat's output marginal before
//! [`walk_postflop`] runs, so the incoming OOP and IP public ranges -- the published ranges of
//! [`crate::publish`], hashed with `core_ranges::hash_scaled` -- are the ones the engine keyed the
//! street's snapshots with. The two seats and their roles come from the street's genuine or
//! admitted projected root ([`snapshot_root`], never a virtual preflop position), recovered by the
//! model at the cutoffs of the street's actions (`crate::snapshot::decision_roots`, ruling 14f-D1:
//! a replay of the truncated hand, hero-gated, the same recovery invalidation keeps snapshots by).
//! A compatible snapshot (spec section 9.2: hand, config and model revision, street, root board,
//! incoming hashes) whose solved root the model cannot reproduce from its `solved_prefix` is not a
//! candidate; among the rest the longest covered prefix of the heads-up street line wins
//! ([`crate::select_snapshot`]'s order). Its reasons are inherited. No snapshot: every seat that
//! acted on the street gets `UnconditionedPriorStreet{street, seat, cause}` once, in order of its
//! first action, and every mass is kept as the earlier streets left it. The cause is `snapshot
//! root not reproducible` when compatible snapshots exist but none of their roots replays;
//! otherwise it follows how the street opened (ruling 15-I2): `multiway prior street` with three
//! or more pot-eligible seats -- even if a later hero decision admitted a projected heads-up root,
//! which is a financial root, not a snapshot -- and `no compatible snapshot` heads-up.
//!
//! # One observed action is one transaction
//!
//! Every non-residual branch starts at the snapshot root with the empty ordinal path (a preflop
//! stop is scoped to preflop); the residual stays frozen. For each observed action of the two root
//! seats (a seat that folded out of an admitted projection is outside the heads-up tree: its
//! actions are disclosed, never applied), each branch is judged at its own ordinal path:
//!
//! 1. the node is exported and lists the action (an inserted observed size is a tree action, so its
//!    solved probability applies, never 1): one on-menu [`crate::condition`] with that action's
//!    column, and the path advances by its menu index;
//! 2. the node is exported and the action is a wager it does not list: spec section 8.4's
//!    translation at the branch's mapped financial parent (below), the branch split over the
//!    interpolated menu sizes, each child advancing by its own menu index;
//! 3. the node is not exported: the action is not applied in that branch (`q` and every mass stay
//!    as conditioned so far), `UnconditionedPriorStreet{"uncovered path <ordinal path>"}` is added,
//!    and the path advances only through an exactly represented skeleton action; an off-menu wager
//!    there has no likelihood and no unique next ordinal, so the branch's navigation freezes for
//!    the rest of the street (spec section 9.2 as amended by revision 6, S14): its `q` and masses
//!    are kept through every later action of the street, each of which is disclosed on it with the
//!    cause that froze it, while branches whose paths stayed defined keep conditioning. The next
//!    street's walk starts every non-residual branch at its own snapshot's root again.
//!
//! The whole generation is expanded by one [`crate::split_batch`] call (ruling 13-R3), capped once
//! ([`crate::cap_branches`], with the walk paths kept aligned) and rescaled once; an action applied
//! in no branch changes nothing, not even by a rescale. If no applying branch has support (spec
//! section 9.2's zero-support rule), the update is rejected: every pre-action `q` and mass is kept,
//! `UnconditionedPriorStreet{"zero support after <action>"}` is added, and navigation stays
//! truthful without a guessed action -- an on-menu observed action still advances its branch,
//! while a translated wager freezes its branch (only an unobserved menu size could advance it). A
//! branch records on its `translated` history every action it navigated: the observed action on a
//! menu (applied or not) and the mapped menu size of a translated wager.
//!
//! At the walk's output boundary, after the street's actions, a residual the cap created or
//! extended is disclosed as `BranchResidual{seat: hero, residual_mass_pct, cause: "cap"}` with
//! its share of the final weights (spec section 8.4, ruling 15-I1), replacing any share an earlier
//! boundary recorded.
//!
//! # The interpolation coefficients
//!
//! A translated wager's pot fraction is taken at the branch's **mapped financial parent**: the
//! selected snapshot's root (pot, dead money, stacks) with the branch's mapped heads-up line to the
//! node -- the menu actions along its ordinal path -- replayed by `core_model::replay_root`. Its
//! `Derived` gives the actor's street contribution `own`, the call `facing - own` and the pot
//! before the call, and Task 10's `wager_fraction` / `menu_fractions` / `interpolate` give
//! `s = (to - own - call) / (pot + call)`, the menu fractions and
//! `f_A = (B - s)(1 + A) / ((B - A)(1 + s))`. `crate::covered_prefix` locates the same children
//! with `interpolate` over the chip amounts `to - (own + call)` (ruling 14-D1): at one node every
//! observed and menu size shares the positive divisor `pot + call`, so the two scales order, equate
//! and bracket the sizes identically -- the same exact match, clamp or bracketing pair, hence the
//! same children -- while only the pot fractions give spec section 8.4's coefficients, which the
//! walk uses.

use crate::branches::{cap_branches_with, rescale, residual_reason, split_batch, zero_reason, BatchSplit, BranchChoice, HistoryBranch};
use crate::preflop::{board_mask, public_range, translation_reason, ReplayInput, ReplayOutput};
use crate::snapshot::{
    compatible, decision_roots, root_board, select_among, snapshot_node_at, street_history, street_number, CompatKey, StreetSnapshot,
};
use core_preflop::{interpolate, menu_fractions, wager_fraction, Interpolation};
use proto::worker::NodeStrategy;
use proto::{
    index_materialized, Action, ApproxReason, Derived, HandState, MaterializedIndex, MaterializedNode, OrdinalPath, Seat, Street, StreetRootSnapshot,
    UnsupportedReason, COMBOS,
};

/// No compatible snapshot for a street that opened heads-up (spec section 9.3): none registered,
/// or none for these incoming ranges. ReplayInput carries no engine failure, deadline or
/// no-request provenance, so this is the cause replay can state ([`fallback_cause`]).
const NO_SNAPSHOT: &str = "no compatible snapshot";
/// Compatible snapshots exist, but the model reproduces none of their solved roots: concrete
/// reconstruction provenance, preferred over the fallback cause.
const NOT_REPRODUCIBLE: &str = "snapshot root not reproducible";
/// No compatible heads-up snapshot for a street that opened with three or more pot-eligible seats
/// (brief Step 5, ruling 15-I2), whether or not a hero decision on it later admitted a projected
/// heads-up root: a financial root is not a snapshot ([`fallback_cause`]).
const MULTIWAY: &str = "multiway prior street";
/// The action of a seat that folded out of the admitted projected root the snapshot was solved on.
const OUTSIDE_ROOT: &str = "not in the heads-up street root";
/// The residual's walk path: frozen, never conditioned.
const RESIDUAL: &str = "residual";

/// Spec section 9.2 case 3's reason: `seat`'s action at the unexported node at ordinal `path` is
/// not applied -- `UnconditionedPriorStreet{street, seat, "uncovered path <path>"}`, the path
/// written as a list (`[]` for the root, `[0, 1]` below it).
pub fn uncovered(street: Street, seat: Seat, path: &[u8]) -> ApproxReason {
    ApproxReason::UnconditionedPriorStreet { street, seat, cause: format!("uncovered path {path:?}") }
}

/// The financial root `snapshot` was solved on, recovered from its solved prefix through the model,
/// never from a later pot: the street root of the hero decision in `state` whose root history
/// equals `snapshot.provenance.solved_prefix` (the genuine heads-up root, or the admitted spec
/// section 10.2 projection with the folded seats' dead money), with its history cleared. Every
/// cutoff of the street's actual actions is tried, so a projection is recovered with the folded
/// seats' intervening actions rather than by truncating at the heads-up prefix's length.
///
/// Built on the same recovery [`crate::SnapshotStore::invalidate`] keeps a snapshot by
/// (`decision_roots`, ruling 14f-D1): a snapshot invalidation keeps is exactly one this reproduces,
/// and the root reproduced is the solved one. It is hero-gated like that recovery -- a cutoff is a
/// decision only with hero's cards on record (ruling 14f-C2) -- though hero's cards never enter the
/// root. A snapshot whose root board is not `state`'s board for its street, or whose prefix no
/// cutoff reproduces, is `UnsupportedHistory{"snapshot root not reproducible"}`; chips are never
/// adjusted to make a root fit.
pub fn snapshot_root(state: &HandState, snapshot: &StreetSnapshot) -> Result<StreetRootSnapshot, UnsupportedReason> {
    let street = snapshot.key.street;
    let roots = if snapshot.key.root_board == root_board(state, street) { decision_roots(state, street) } else { vec![] };
    reproduce(&roots, snapshot).ok_or_else(|| UnsupportedReason::UnsupportedHistory { reason: NOT_REPRODUCIBLE.into() })
}

/// Replays the completed postflop `street` of `input.state` into `output` (see the module docs).
/// `output` must stand at the street's root: the earlier streets replayed and the street's root
/// board blocked ([`crate::block_and_rescale`]), as [`crate::replay`] leaves it. Only the street's
/// own recorded actions are read, and nothing is ever solved. At its output boundary, after the
/// street's actions, the cap residual is disclosed with its current share
/// (`BranchResidual{seat: hero, residual_mass_pct, cause: "cap"}`, see the module docs).
///
/// # Panics
/// Always, if `street` is the preflop, if `input.snapshots` mixes model revisions (the caller passes
/// one identity's snapshots, `SnapshotStore::for_identity`), if the model recovers two different
/// seat pairs for one street, if an exported node disagrees with its skeleton node or has other than
/// 1326 rows, or through the kernel's own invariant checks ([`crate::split_batch`] and its
/// `condition`, [`crate::cap_branches`], [`crate::rescale`], [`crate::residual_reason`]).
pub fn walk_postflop(input: &ReplayInput, street: Street, output: &mut ReplayOutput) {
    assert!(street != Street::Preflop, "walk_postflop: the preflop street is walked by walk_preflop");
    // A preflop stop is scoped to the preflop street (spec section 9.3), and no preflop node key
    // names a postflop decision: every non-residual branch restarts at the street root.
    for b in output.branches.iter_mut() {
        if !b.residual {
            b.stopped = None;
        }
        for s in &mut b.seats {
            s.node = None;
        }
    }
    let history = street_history(input.state, street);
    match choose(input, street, &history, output) {
        Ok(chosen) => walk_street(street, &history, &chosen, output),
        Err(cause) => {
            for seat in acting_seats(&history) {
                output.reasons.push(ApproxReason::UnconditionedPriorStreet { street, seat, cause: cause.into() });
            }
        }
    }
    disclose_cap(output, input.state.hero);
}

/// Walks `history` through the `chosen` snapshot (see the module docs).
fn walk_street(street: Street, history: &[(Seat, Action)], chosen: &Chosen, output: &mut ReplayOutput) {
    for r in &chosen.snapshot.reasons {
        note(output, r.clone());
    }
    let walk = Walk { street, snapshot: chosen.snapshot, root: &chosen.root, tree: index_materialized(&chosen.snapshot.tree.materialized) };
    let mut paths: Vec<WalkPath> =
        output.branches.iter().map(|b| if b.residual { WalkPath::frozen(RESIDUAL) } else { WalkPath::at(vec![]) }).collect();
    for (seat, action) in history {
        if *seat != walk.root.oop && *seat != walk.root.ip {
            note(output, ApproxReason::UnconditionedPriorStreet { street, seat: *seat, cause: OUTSIDE_ROOT.into() });
            continue;
        }
        paths = walk.apply(output, paths, *seat, action);
    }
}

/// Spec section 8.4's cap disclosure at a replay output boundary (rulings 15-I1, 15-N1, 15-Q5):
/// exactly one `BranchResidual{seat: hero, residual_mass_pct, cause: "cap"}`, the current share of
/// the branch list ([`crate::residual_reason`]) -- the frozen residual's share grows as later
/// evidence shrinks the live weights, so a share recorded earlier is never kept. Every hero cap
/// disclosure already on the output (an earlier boundary's, or one a snapshot inherited with
/// another share) collapses into this one, at the first one's position; with no residual, none
/// remains. A `BranchResidual` with another cause or seat is untouched, and so are the branches.
/// The postflop walk calls it at its exit and `replay` at its own output boundary.
pub(crate) fn disclose_cap(output: &mut ReplayOutput, hero: Seat) {
    let is_cap = |r: &ApproxReason| matches!(r, ApproxReason::BranchResidual { seat, cause, .. } if *seat == hero && cause == "cap");
    let first = output.reasons.iter().position(is_cap);
    // Nothing before `first` is removed, so the index stays valid.
    output.reasons.retain(|r| !is_cap(r));
    if let Some(r) = residual_reason(&output.branches, hero) {
        output.reasons.insert(first.unwrap_or(output.reasons.len()), r);
    }
}

// ---------------------------------------------------------------------------------------------
// Selection.
// ---------------------------------------------------------------------------------------------

/// The snapshot selected for a street and the financial root it was solved on.
struct Chosen<'a> {
    snapshot: &'a StreetSnapshot,
    root: StreetRootSnapshot,
}

/// Selects the street's snapshot (see the module docs), or names why there is none.
fn choose<'a>(input: &ReplayInput<'a>, street: Street, history: &[(Seat, Action)], output: &ReplayOutput) -> Result<Chosen<'a>, &'static str> {
    let board = root_board(input.state, street);
    let model_revision = model_revision_of(input.snapshots);
    let roots = decision_roots(input.state, street);
    let Some(first) = roots.first() else {
        let for_this_street = input.snapshots.iter().any(|s| {
            s.key.hand_id == input.state.hand_id
                && s.key.config_revision == input.state.config.config_revision
                && s.key.street == street
                && s.key.root_board == board
        });
        return Err(if for_this_street { NOT_REPRODUCIBLE } else { fallback_cause(input.state, street) });
    };
    // Every hero decision root of one street names the same two seats: pot eligibility only
    // shrinks within a street, and each root has exactly two eligible seats.
    let (oop, ip) = (first.oop, first.ip);
    for r in &roots {
        assert!(
            (r.oop, r.ip) == (oop, ip),
            "walk_postflop: the model recovered the {street:?} roots ({:?}, {:?}) and ({oop:?}, {ip:?})",
            r.oop,
            r.ip
        );
    }
    let Some(model_revision) = model_revision else { return Err(fallback_cause(input.state, street)) };
    let mask = board_mask(&board);
    let hash = |seat: Seat| core_ranges::hash_scaled(&public_range(&output.branches, seat, &mask));
    let key = CompatKey {
        hand_id: input.state.hand_id,
        config_revision: input.state.config.config_revision,
        model_revision,
        street,
        root_board: board.clone(),
        root_range_hashes: [hash(oop), hash(ip)],
    };
    let candidates: Vec<&StreetSnapshot> = input.snapshots.iter().filter(|s| compatible(&s.key, &key)).collect();
    if candidates.is_empty() {
        return Err(fallback_cause(input.state, street));
    }
    // Coverage is measured on the heads-up line, the domain the snapshot's tree was built in.
    let heads_up: Vec<(Seat, Action)> = history.iter().copied().filter(|(s, _)| *s == oop || *s == ip).collect();
    let selected =
        select_among(candidates.into_iter().filter(|s| reproduce(&roots, s).is_some()), &key, &heads_up).ok_or(NOT_REPRODUCIBLE)?;
    let root = reproduce(&roots, selected).expect("a selected snapshot reproduces its root");
    Ok(Chosen { snapshot: selected, root })
}

/// The root among `roots` whose history is `snapshot`'s solved prefix, its history cleared.
fn reproduce(roots: &[StreetRootSnapshot], snapshot: &StreetSnapshot) -> Option<StreetRootSnapshot> {
    roots.iter().find(|r| r.history == snapshot.provenance.solved_prefix).map(|r| StreetRootSnapshot { history: vec![], ..r.clone() })
}

/// The model revision of the supplied snapshots (they carry the one [`ReplayInput`] lacks), `None`
/// when there are none.
///
/// # Panics
/// Always, if two snapshots disagree: the engine passes `SnapshotStore::for_identity` of the active
/// decision, so a mixed slice is a caller bug.
fn model_revision_of(snapshots: &[StreetSnapshot]) -> Option<u32> {
    let first = snapshots.first()?.key.model_revision;
    for (i, s) in snapshots.iter().enumerate() {
        assert!(
            s.key.model_revision == first,
            "walk_postflop: snapshot {i} has model revision {}, snapshot 0 has {first}; replay takes one identity's snapshots",
            s.key.model_revision
        );
    }
    Some(first)
}

/// The one cause for a street with no compatible heads-up snapshot and no more concrete
/// provenance (ruling 15-I2): `multiway prior street` when the street OPENED with three or more
/// pot-eligible seats -- whatever its later hero decisions admitted -- and `no compatible
/// snapshot` when it opened heads-up.
fn fallback_cause(state: &HandState, street: Street) -> &'static str {
    if opened_multiway(state, street) {
        MULTIWAY
    } else {
        NO_SNAPSHOT
    }
}

/// Whether `street` opened with three or more pot-eligible seats: the dealt seats that did not
/// fold on an earlier street (an all-in seat stays pot-eligible).
fn opened_multiway(state: &HandState, street: Street) -> bool {
    let folded_before =
        state.actions.iter().filter(|a| street_number(a.street) < street_number(street) && a.action == Action::Fold).count();
    state.dealt.len().saturating_sub(folded_before) >= 3
}

/// The seats that acted in `history`, once each, in order of their first action.
fn acting_seats(history: &[(Seat, Action)]) -> Vec<Seat> {
    let mut seats: Vec<Seat> = Vec::new();
    for (seat, _) in history {
        if !seats.contains(seat) {
            seats.push(*seat);
        }
    }
    seats
}

/// Adds `r` unless an identical reason is already recorded.
fn note(output: &mut ReplayOutput, r: ApproxReason) {
    if !output.reasons.contains(&r) {
        output.reasons.push(r);
    }
}

// ---------------------------------------------------------------------------------------------
// The walk.
// ---------------------------------------------------------------------------------------------

/// Where one branch stands in the selected snapshot's tree: its ordinal path, or `None` once its
/// navigation froze for the rest of the street, with why (`unknown_cause`, set exactly then).
#[derive(Clone, Debug, PartialEq)]
struct WalkPath {
    ordinal: Option<OrdinalPath>,
    unknown_cause: Option<String>,
}

impl WalkPath {
    fn at(ordinal: OrdinalPath) -> Self {
        WalkPath { ordinal: Some(ordinal), unknown_cause: None }
    }

    fn frozen(cause: impl Into<String>) -> Self {
        WalkPath { ordinal: None, unknown_cause: Some(cause.into()) }
    }
}

/// Where a branch's walk path goes once the observed action has been decided for it.
enum Next {
    /// Unchanged: the residual, or navigation already frozen.
    Stay,
    /// One next ordinal path: an action on the node's menu, applied or (uncovered) not.
    Advance(OrdinalPath),
    /// A translated split: the ordinal path of each choice, in choice order.
    Children(Vec<OrdinalPath>),
    /// No next ordinal path can be justified; navigation freezes with this cause.
    Freeze(String),
}

/// What one observed action does to one branch: the kernel's choice and the path it leads to.
struct Plan {
    choice: BranchChoice,
    next: Next,
}

impl Plan {
    fn keep(next: Next) -> Self {
        Plan { choice: BranchChoice::Keep, next }
    }

    /// The walk path of output branch `b`, which came from this plan's branch (`choice`: its split
    /// choice, if a child), after the transaction. A branch that navigated an uncovered skeleton
    /// action records it on its `translated` history, as an applied action records itself.
    ///
    /// # Panics
    /// Always, if a split child has no choice index (the kernel's `origin` always gives one).
    fn next_path(&self, w: &WalkPath, choice: Option<usize>, b: &mut HistoryBranch, seat: Seat, action: &Action) -> WalkPath {
        match &self.next {
            Next::Stay => w.clone(),
            Next::Advance(path) => {
                if matches!(self.choice, BranchChoice::Keep) {
                    b.translated.push((seat, *action));
                }
                WalkPath::at(path.clone())
            }
            Next::Children(paths) => WalkPath::at(paths[choice.expect("walk_postflop: a split child carries its choice index")].clone()),
            Next::Freeze(cause) => WalkPath::frozen(cause.clone()),
        }
    }
}

/// The selected snapshot being walked for one street.
struct Walk<'s> {
    street: Street,
    snapshot: &'s StreetSnapshot,
    root: &'s StreetRootSnapshot,
    tree: MaterializedIndex<'s>,
}

impl Walk<'_> {
    /// The seat a skeleton node's actor names (`"oop"` or `"ip"`) at this root.
    fn seat_of(&self, actor: &str) -> Option<Seat> {
        match actor {
            "oop" => Some(self.root.oop),
            "ip" => Some(self.root.ip),
            _ => None,
        }
    }

    fn unconditioned(&self, seat: Seat, cause: String) -> ApproxReason {
        ApproxReason::UnconditionedPriorStreet { street: self.street, seat, cause }
    }

    /// Applies one observed action of a root seat to every branch as one transaction and returns
    /// the branches' next walk paths (index-aligned with `output.branches`).
    fn apply(&self, output: &mut ReplayOutput, paths: Vec<WalkPath>, seat: Seat, action: &Action) -> Vec<WalkPath> {
        let mut notes = Vec::new();
        let plans: Vec<Plan> = output.branches.iter().zip(&paths).map(|(b, w)| self.plan(b, w, seat, action, &mut notes)).collect();
        for r in notes {
            note(output, r);
        }
        // Ruling 13-R3: the complete generation, each branch with its own choice, in ONE kernel call.
        let choices: Vec<BranchChoice> = plans.iter().map(|p| p.choice.clone()).collect();
        let BatchSplit { mut branches, origin, applying, supported } = split_batch(&output.branches, seat, &choices);
        if applying && !supported {
            // Spec section 9.2's zero-support rule: the update is rejected and every pre-action `q`
            // and mass is kept. Navigation stays truthful without a guessed action (as preflop,
            // ruling 13-R3's transaction): an observed action on the node's menu still advances
            // along it, a translated wager -- which could only advance along an unobserved menu
            // size -- freezes its branch, and every other branch follows its own plan.
            let cause = format!("zero support after {action:?}");
            let next = output
                .branches
                .iter_mut()
                .zip(&plans)
                .zip(&paths)
                .map(|((b, plan), w)| match (&plan.choice, &plan.next) {
                    (BranchChoice::Retain { .. }, Next::Advance(path)) => {
                        b.translated.push((seat, *action));
                        WalkPath::at(path.clone())
                    }
                    (BranchChoice::Split(_), _) => WalkPath::frozen(cause.clone()),
                    _ => plan.next_path(w, None, b, seat, action),
                })
                .collect();
            note(output, zero_reason(self.street, seat, action));
            return next;
        }
        let mut next: Vec<WalkPath> =
            branches.iter_mut().zip(&origin).map(|(b, &(i, choice))| plans[i].next_path(&paths[i], choice, b, seat, action)).collect();
        output.branches = branches;
        if supported {
            cap_branches_with(&mut output.branches, &mut next, || WalkPath::frozen(RESIDUAL));
            rescale(&mut output.branches, &mut output.log_reach);
        }
        next
    }

    /// Decides what `seat`'s `action` does to branch `b`, which stands at `w` (see the module docs).
    fn plan(&self, b: &HistoryBranch, w: &WalkPath, seat: Seat, action: &Action, notes: &mut Vec<ApproxReason>) -> Plan {
        if b.residual {
            return Plan::keep(Next::Stay);
        }
        let Some(path) = w.ordinal.as_ref() else {
            // Navigation froze earlier on this street (spec section 9.2, S14): every later action
            // on this branch stays unapplied, and says why.
            let cause = w.unknown_cause.clone().expect("walk_postflop: a frozen walk path carries its cause");
            notes.push(self.unconditioned(seat, cause));
            return Plan::keep(Next::Stay);
        };
        let mut freeze = |cause: String| {
            notes.push(self.unconditioned(seat, cause.clone()));
            Plan::keep(Next::Freeze(cause))
        };
        let Some(node) = self.tree.get(path.as_slice()).copied() else {
            return freeze(format!("path left the skeleton at {path:?}"));
        };
        if self.seat_of(&node.actor) != Some(seat) {
            return freeze(format!("the skeleton's {} acts at {path:?}", node.actor));
        }
        let index = node.actions.iter().position(|a| a == action);
        match (snapshot_node_at(self.snapshot, path), index) {
            // (1) Exported and on the menu: condition once with the action's solved column.
            (Some(strategy), Some(i)) => match child(path, i) {
                Some(next) => Plan { choice: BranchChoice::Retain { action: *action, p: likelihood(strategy, node, path, i) }, next: Next::Advance(next) },
                None => freeze(format!("path left the skeleton at {path:?}")),
            },
            // (2) Exported, but the wager is off this node's menu: translate and split.
            (Some(strategy), None) => self.translate(path, node, strategy, seat, action, notes),
            // (3) Not exported: never applied. An exact skeleton action still advances the path.
            (None, Some(i)) => {
                notes.push(uncovered(self.street, seat, path));
                match child(path, i) {
                    Some(next) => Plan::keep(Next::Advance(next)),
                    None => Plan::keep(Next::Freeze(format!("uncovered path {path:?}"))),
                }
            }
            (None, None) => {
                notes.push(uncovered(self.street, seat, path));
                Plan::keep(Next::Freeze(format!("uncovered path {path:?}")))
            }
        }
    }

    /// Case (2): `seat`'s off-menu `action` at the exported node at `path`, translated over the
    /// node's own wager sizes at the branch's mapped financial parent (see the module docs) and
    /// split over the interpolated sizes; the branch freezes, never guessing, when the action is
    /// not a wager or has no pot fraction or interpolation there.
    fn translate(&self, path: &[u8], node: &MaterializedNode, strategy: &NodeStrategy, seat: Seat, action: &Action, notes: &mut Vec<ApproxReason>) -> Plan {
        let mapped = wager_to(action).and_then(|to| {
            let (s, menu, t) = interpolation_at(node, &self.mapped_parent(path, seat)?, seat, to)?;
            let children = t.choices.iter().map(|&(i, _)| child(path, i)).collect::<Option<Vec<OrdinalPath>>>()?;
            Some((s, menu, t, children))
        });
        let Some((s, menu, t, children)) = mapped else {
            let cause = match wager_to(action) {
                Some(_) => format!("unmappable size at {path:?}"),
                None => format!("unmappable action {action:?} at {path:?}"),
            };
            notes.push(self.unconditioned(seat, cause.clone()));
            return Plan::keep(Next::Freeze(cause));
        };
        notes.push(translation_reason(self.street, seat, s, &menu, &t));
        Plan {
            choice: BranchChoice::Split(t.choices.iter().map(|&(i, f)| (node.actions[i], f, likelihood(strategy, node, path, i))).collect()),
            next: Next::Children(children),
        }
    }

    /// The money at the node at `path` in the mapped line: the snapshot's financial root with the
    /// menu actions along `path` as its heads-up history, replayed by `core_model::replay_root`;
    /// `None` if that line does not replay to `seat`'s decision.
    fn mapped_parent(&self, path: &[u8], seat: Seat) -> Option<Derived> {
        let mut history = Vec::with_capacity(path.len());
        for k in 0..path.len() {
            let node = self.tree.get(&path[..k])?;
            history.push((self.seat_of(&node.actor)?, *node.actions.get(usize::from(path[k]))?));
        }
        let money = core_model::replay_root(&StreetRootSnapshot { history, ..self.root.clone() }).ok()?;
        (money.to_act == Some(seat)).then_some(money)
    }
}

/// Spec section 8.4's interpolation of `seat`'s wager to `to` over `node`'s own wager sizes at the
/// mapped financial parent `money`: `own` is the seat's street contribution, `call = facing - own`,
/// and the pot is `money.pot`, the pot before the call (Task 10's `wager_fraction`,
/// `menu_fractions`, `interpolate`). Returns the observed fraction, the menu fractions and the
/// interpolation; `None` where a size has no pot fraction there or nothing interpolates.
fn interpolation_at(node: &MaterializedNode, money: &Derived, seat: Seat, to: u32) -> Option<(f64, Vec<(usize, f64)>, Interpolation)> {
    let own = *money.committed_this_street.get(usize::from(seat.0))?;
    let call = money.facing.checked_sub(own)?;
    let s = wager_fraction(to, own, call, money.pot)?;
    let menu = menu_fractions(&node.actions, own, call, money.pot)?;
    let t = interpolate(s, &menu)?;
    Some((s, menu, t))
}

/// `path` extended by menu index `i`; `None` for an index an ordinal path cannot hold.
fn child(path: &[u8], i: usize) -> Option<OrdinalPath> {
    let i = u8::try_from(i).ok()?;
    let mut next = path.to_vec();
    next.push(i);
    Some(next)
}

/// The amount a wager-sized action takes its actor's street contribution to; `None` for fold,
/// check and call.
fn wager_to(action: &Action) -> Option<u32> {
    match action {
        Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => Some(*to),
        Action::Fold | Action::Check | Action::Call => None,
    }
}

/// Menu action `a`'s solved likelihood column over 1326 combos, as `f64`: `P(a | c)` from the
/// exported strategy, and 0 for a combo the solve left unavailable (no likelihood is invented).
///
/// # Panics
/// Always, if the exported node does not list its skeleton node's menu or does not hold one row of
/// that width per combo (registration validates both, spec section 4.5).
fn likelihood(strategy: &NodeStrategy, node: &MaterializedNode, path: &[u8], a: usize) -> Vec<f64> {
    assert!(
        strategy.actions == node.actions,
        "walk_postflop: the exported node at {path:?} lists {:?}, its skeleton node {:?}",
        strategy.actions,
        node.actions
    );
    assert!(
        strategy.probs.len() == COMBOS && strategy.available.len() == COMBOS,
        "walk_postflop: the exported node at {path:?} has {} probability rows and {} availability flags, expected {COMBOS}",
        strategy.probs.len(),
        strategy.available.len()
    );
    strategy
        .probs
        .iter()
        .zip(&strategy.available)
        .enumerate()
        .map(|(c, (row, &available))| {
            assert!(
                row.len() == node.actions.len(),
                "walk_postflop: the exported node at {path:?} has {} probabilities for combo {c}, expected {}",
                row.len(),
                node.actions.len()
            );
            if available {
                f64::from(row[a])
            } else {
                0.0
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{translated_children, wager_level, SnapshotKey, SnapshotProvenance};
    use proto::{Card, DecisionIdentity, EffectiveTree};
    use std::collections::BTreeMap;

    fn node(path: &[u8], actor: &str, actions: Vec<Action>) -> MaterializedNode {
        let terminal_pots = vec![None; actions.len()];
        MaterializedNode { path: path.to_vec(), street: Street::Flop, actor: actor.into(), actions, terminal_pots }
    }

    /// A snapshot holding only the tree `materialized` (the walk's money and actor lookups read
    /// nothing else).
    fn bare(materialized: Vec<MaterializedNode>) -> StreetSnapshot {
        StreetSnapshot {
            key: SnapshotKey {
                hand_id: 1,
                config_revision: 1,
                model_revision: 0,
                street: Street::Flop,
                root_board: vec![Card(46), Card(21), Card(0)],
                root_range_hashes: [[0; 32]; 2],
                tree_signature: "t".into(),
            },
            provenance: SnapshotProvenance {
                identity_at_solve: DecisionIdentity { hand_id: 1, hand_revision: 0, decision_id: 0, config_revision: 1, model_revision: 0 },
                solved_prefix: vec![],
                origin: "live".into(),
            },
            tree: EffectiveTree {
                rules_version: 3,
                template_id: "t".into(),
                root_street: Street::Flop,
                menus: BTreeMap::new(),
                add_allin_threshold: 0.0,
                force_allin_threshold: 0.0,
                merging_threshold: 0.0,
                wager_cap: 3,
                inserted: vec![],
                materialized,
            },
            nodes: vec![],
            covered_paths: vec![],
            exploitability_chips: 0.0,
            reasons: vec![],
        }
    }

    /// Ruling 14-D1's equivalence at the walk's own money: at every node of a mapped line (opening
    /// and facing, with dead money in the root), the chip-amount level `covered_prefix` measures
    /// sizes from is the mapped financial parent's `own + call`, and the children the walk's
    /// pot-fraction interpolation gives a nonzero coefficient are exactly `covered_prefix`'s
    /// children, for every observed size from 0 to 1,000 chips of each wager kind.
    #[test]
    fn the_walks_children_are_the_children_covered_prefix_counts() {
        let (bet, raise, all_in) = (|to| Action::Bet { to }, |to| Action::Raise { to }, Action::AllIn { to: 950 });
        let snapshot = bare(vec![
            node(&[], "oop", vec![Action::Check, bet(50), bet(100), all_in]),
            node(&[0], "ip", vec![Action::Check, bet(33), bet(75), bet(100), all_in]),
            node(&[1], "ip", vec![Action::Fold, Action::Call, raise(150), raise(300), all_in]),
            node(&[0, 2], "oop", vec![Action::Fold, Action::Call, raise(250), raise(400), all_in]),
            node(&[1, 3], "oop", vec![Action::Fold, Action::Call, raise(700), all_in]),
        ]);
        // A projected root: 100 settled, 30 dead on the street, 950 behind each.
        let root = StreetRootSnapshot {
            street: Street::Flop,
            board: snapshot.key.root_board.clone(),
            oop: Seat(1),
            ip: Seat(5),
            pot_root: 100,
            stack_oop_root: 950,
            stack_ip_root: 950,
            dead_this_street: 30,
            projected_from: 3,
            history: vec![],
            bb_chips: 10,
        };
        let walk = Walk { street: Street::Flop, snapshot: &snapshot, root: &root, tree: index_materialized(&snapshot.tree.materialized) };
        let paths: [&[u8]; 5] = [&[], &[0], &[1], &[0, 2], &[1, 3]];
        let mut compared = 0;
        for path in paths {
            let node = *walk.tree.get(path).expect("a skeleton node");
            let seat = walk.seat_of(&node.actor).expect("oop or ip");
            let money = walk.mapped_parent(path, seat).expect("the mapped line replays to the node's actor");
            let level = wager_level(&walk.tree, path, Street::Flop).expect("a skeleton path");
            let own = money.committed_this_street[usize::from(seat.0)];
            assert_eq!(own + (money.facing - own), level, "own + call is the wager level at {path:?}");
            for to in 0..=1_000u32 {
                for observed in [Action::Bet { to }, Action::Raise { to }, Action::AllIn { to }] {
                    let walked = wager_to(&observed)
                        .and_then(|to| interpolation_at(node, &money, seat, to))
                        .map(|(_, _, t)| t.choices.iter().filter(|(_, f)| *f > 0.0).map(|(i, _)| *i).collect::<Vec<usize>>());
                    assert_eq!(walked, translated_children(node, level, &observed), "{path:?}, observed {observed:?}");
                    compared += 1;
                }
            }
        }
        assert_eq!(compared, 5 * 1_001 * 3);
        // The fraction is taken over the whole pot, dead money included: the IP faces 50 into 100 +
        // 30 dead, so a raise to 150 is (150 - 50) / (180 + 50).
        let money = walk.mapped_parent(&[1], Seat(5)).expect("the IP's node");
        assert_eq!((money.pot, money.facing), (180, 50));
        let (s, _, _) = interpolation_at(walk.tree.get(&[1][..]).expect("a node"), &money, Seat(5), 150).expect("a raise");
        assert_eq!(s, 100.0 / 230.0);
        // A line that does not replay to the node's actor has no mapped parent.
        assert!(walk.mapped_parent(&[1], Seat(1)).is_none());
    }

    /// The walk path's two states: defined (no cause) or frozen (a cause, no ordinal).
    #[test]
    fn a_walk_path_is_defined_or_frozen_with_its_cause() {
        assert_eq!(WalkPath::at(vec![0, 2]), WalkPath { ordinal: Some(vec![0, 2]), unknown_cause: None });
        assert_eq!(WalkPath::frozen("uncovered path [0]"), WalkPath { ordinal: None, unknown_cause: Some("uncovered path [0]".into()) });
        assert_eq!(child(&[0], 2), Some(vec![0, 2]));
        assert_eq!(child(&[0], 256), None, "an index an ordinal path cannot hold");
    }
}
