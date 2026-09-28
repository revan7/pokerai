//! Preflop replay with frozen missing-node stops (spec sections 8.3, 8.4, 9.1-9.3; plan 3 Task 13).
//!
//! [`replay`] starts from one branch with `q = 1` and uniform masses for every dealt seat, applies
//! every observed preflop action once in every live branch ([`walk_preflop`],
//! [`apply_preflop_action`]), walks each completed postflop street through its selected snapshot
//! ([`walk_postflop`], P3.T15), blocks the board on each street root's output marginal
//! ([`block_and_rescale`]) and publishes every dealt seat's range ([`publish`]).
//!
//! # One observed preflop action is one transaction
//!
//! For every branch of the shared list, in order:
//!
//! - a residual or stopped branch is copied through unchanged (frozen);
//! - a live branch looks up the actor's node under **its own translated history**, with the exact
//!   source step it chose at each translated edge carried alongside (the observed prefix still
//!   supplies depth, eligibility, rake, roles and the short-handed folds). No node (spec section
//!   9.3): the branch stops for the rest of the preflop street -- `q` and every mass frozen, no
//!   node for any seat, reason `UnconditionedPriorStreet{Preflop, seat, "missing node <key>"}` --
//!   and is never resumed at a later present node;
//! - the observed action on the node's menu (section 8.3's size rule): one [`condition`](crate::branches::condition) with
//!   that action's column, the branch keeping its id and recording the **observed** action (only
//!   translated wagers are replaced, section 8.4);
//! - an off-menu wager: section 8.4's pseudo-harmonic interpolation at the branch's mapped
//!   **source** parent, disclosed as `BetTranslation`, and the branch is split over the mapped
//!   menu sizes; a size with no pot fraction there stops the branch (`unmappable size at <key>`).
//!
//! Every branch's choice is decided first, then the whole pre-action generation is expanded by one
//! [`split_batch`] call (below). If no applying branch has support (spec section 9.2's
//! zero-support rule), the update is rejected: every `q` and mass keeps its pre-action value and
//! `UnconditionedPriorStreet{"zero support after <action>"}` is added. Navigation is still kept
//! truthful without a guessed action: a branch whose observed action was on its menu advances along
//! that observed action (no likelihood), and a branch whose action was an off-menu wager -- which
//! could only advance along an unobserved menu size -- stops. Otherwise the batch replaces the
//! list, the cap runs once ([`cap_branches`]) and every seat is rescaled once ([`rescale`]).
//!
//! # Source-step identity (ruling 13-R1)
//!
//! A translated edge is recorded as the chip action its menu size expands to, and a chip amount
//! cannot always name its source size: at a 3-chip big blind both 2.5 and 2.6 bb are
//! `Raise{to: 8}`. The walk therefore carries, per branch, the exact source step it chose at each
//! translated edge and looks every node up with it
//! ([`core_preflop::PreflopInvocation::answer_history_sourced`]); an on-menu observed action is
//! kept as observed, so its own resolution is the one it was conditioned with. The chip-only
//! public entry points ([`query_translated`], and a standalone [`apply_preflop_action`], which
//! holds only the branches' chip histories) recover an edge from its chip amount only when exactly
//! one source size rounds to it; otherwise the lookup is an explicit unresolved missing path and
//! the branch stops on it -- never a silently chosen other edge.
//!
//! # Source-unit interpolation (ruling 13-R2)
//!
//! The pot fractions of section 8.4 are taken at the source node's own parent: the source's posts
//! (0.5 and 1 source unit, by virtual role -- behind a straddle the physical SB's post is not in
//! the virtual tree, spec section 8.3) and every source step of the branch's resolved key, in
//! source units, before any chip rounding. The observed amount and every menu size are expressed
//! in one exact integer scale, milli-chips (a source amount of `x` thousandths of a unit is
//! `x * unit`, an actual chip amount `c` is `c * 1000`), so fractional source sizes survive; an
//! `AllIn` menu entry keeps the actor's actual maximum (P3.T9).
//!
//! # One generation per observed action (ruling 13-R3)
//!
//! The kernel's [`split_batch`] takes the complete pre-action generation with one choice per
//! branch -- kept (frozen), stopped, retained on the menu, or split over its own menu -- and
//! allocates ids and remaps parents once, so several branches splitting at the same action with
//! different menus keep every parent link even across the `u8` id compaction; zero-support
//! removals are part of the same pass.

use crate::branches::{
    cap_branches, initial, marginal, missing_reason, range_output, rescale, split_batch, stop_branch, zero_reason, BranchChoice,
    HistoryBranch,
};
use crate::postflop::walk_postflop;
use crate::snapshot::StreetSnapshot;
use core_preflop::{
    interpolate, menu_step_index, ExpandedNode, Interpolation, PreflopAnswer, PreflopInvocation, PreflopNode, PreflopNodeKey,
    PreflopStep, PreflopStore,
};
use proto::{Action, ApproxReason, Card, HandConfig, HandState, Position, Range1326, Seat, Street, UnsupportedReason, COMBOS};
use std::collections::BTreeMap;

/// Everything one replay reads (spec section 9.1). `snapshots` are the street solutions
/// registered for this hand, one identity's (see [`replay`]'s precondition); the postflop walk
/// ([`walk_postflop`]) selects one per completed street.
#[derive(Clone, Copy)]
pub struct ReplayInput<'a> {
    pub cfg: &'a HandConfig,
    pub state: &'a HandState,
    pub store: &'a PreflopStore,
    pub snapshots: &'a [StreetSnapshot],
}

/// One replay's result (spec section 9.1).
///
/// - `ranges`: indexed by seat id, length 6: every dealt seat's public marginal at the current
///   street root with the board removed, narrowed to `f32` at this boundary only; `None` for a
///   vacant seat.
/// - `branches`: the shared list of spec section 8.4 (at most 4 live branches plus 1 residual),
///   every dealt seat's masses inside, folded seats' included.
/// - `folded_ranges`: the published ranges of the dealt seats that folded, in increasing seat id.
/// - `log_reach`: indexed by seat id, length 6: the sum of `ln` of every maximum removed from that
///   seat's marginal; 0 for a vacant seat.
/// - `reasons`: every approximation the replay made, in the order it made them.
/// - `unsupported`: set when the ranges cannot be used at all (`InvalidRanges`, or a preflop lookup
///   the hand's format or history cannot have).
#[derive(Clone, Debug)]
pub struct ReplayOutput {
    pub ranges: Vec<Option<Range1326>>,
    pub branches: Vec<HistoryBranch>,
    pub folded_ranges: Vec<Range1326>,
    pub log_reach: Vec<f64>,
    pub reasons: Vec<ApproxReason>,
    pub unsupported: Option<UnsupportedReason>,
}

/// Replays `input.state`'s public history into every dealt seat's public range (spec section 9):
/// the start state (one branch, `q = 1`, uniform masses; hero's cards never applied), every
/// preflop action ([`walk_preflop`]), then each completed postflop street in order -- its root
/// blocked ([`block_and_rescale`]) and its observed actions walked through the selected snapshot
/// ([`walk_postflop`]; with none, the street is left unconditioned with its reason) -- and finally
/// the current street's root blocked and every range published ([`publish`]). Only completed
/// streets are walked: for the turn the flop, for the river the flop then the turn, for the flop
/// none. The current street's own postflop actions are never replayed (Plan 2's street-root solve
/// inserts them exactly), and no prior street is ever solved here. A preflop stop is scoped to the
/// preflop street (spec section 9.3), so it clears once the hand has entered a postflop street; the
/// residual never changes.
///
/// Precondition on `input.snapshots` (P3.T14): [`ReplayInput`] has no model revision, so the slice
/// must already be filtered to one hand, config revision and model revision -- the engine passes
/// [`SnapshotStore::for_identity`](crate::SnapshotStore::for_identity) of the active decision, and a
/// baseline caller without the engine passes `model_revision = 0` snapshots only. Replay never
/// derives the active model from an arbitrary snapshot; the engine remains the identity authority.
pub fn replay(input: ReplayInput) -> ReplayOutput {
    let mut output = ReplayOutput {
        ranges: vec![None; 6],
        branches: initial(&input.state.dealt),
        folded_ranges: vec![],
        log_reach: vec![0.0; 6],
        reasons: vec![],
        unsupported: None,
    };
    walk_preflop(&input, &mut output);
    let current = current_street(input.state);
    if current != Street::Preflop {
        clear_preflop_stops(&mut output);
    }
    for street in completed_streets(current) {
        block_and_rescale(&mut output, root_board(input.state, street));
        walk_postflop(&input, street, &mut output);
    }
    block_and_rescale(&mut output, root_board(input.state, current));
    publish(&mut output, input.state);
    output
}

/// Applies every observed preflop action of `input.state`, in order, through
/// [`apply_preflop_action`]'s transaction, stopping at the first postflop action. One walk state
/// serves every action: the mapping memo ([`PreflopInvocation`]), keyed by the observed prefix
/// index, each branch's translated history and the source steps carried along it, and those
/// carried source steps themselves (ruling 13-R1), so every translated edge is navigated as the
/// source size it was mapped to. `output` must be the start state [`replay`] builds (every live
/// branch carries one translated action per applied observed action).
pub fn walk_preflop(input: &ReplayInput, output: &mut ReplayOutput) {
    let mut run = ReplayState::default();
    refresh_nodes(input, output, &mut run, 0);
    for (i, taken) in input.state.actions.iter().enumerate() {
        if taken.street != Street::Preflop {
            break;
        }
        apply_with(input, output, &mut run, i, taken.seat, &taken.action);
    }
}

/// One observed preflop action -- the `prefix_len`-th recorded action, by `seat` -- applied to
/// every branch as one transaction (see the module docs). Returns `false` exactly when the update
/// was rejected by the zero-support rule (some branch applied it and none had support), in which
/// case every `q` and mass is unchanged; `true` otherwise, including when no branch applied it at
/// all (every branch frozen or stopping here). After the transaction, each live branch's seat to
/// act next holds its node key (`SeatMass::node`); every other seat's node is `None`, since its next
/// decision depends on actions not yet observed.
///
/// A standalone call holds only the branches' chip histories (spec section 9.1's
/// `translated`), not the source steps [`walk_preflop`] carries between actions, so its lookups
/// are chip-only (ruling 13-R1): a translated edge whose chip amount is the rounding of two source
/// sizes is an explicit unresolved missing path, and that branch stops on it.
///
/// # Panics
/// Always, if `(seat, observed)` is not the preflop action recorded at `prefix_len`, if a live
/// branch does not carry exactly `prefix_len` translated actions, or through the kernel's own
/// invariant checks ([`split_batch`] and its `condition`, [`cap_branches`], [`rescale`]).
pub fn apply_preflop_action(input: &ReplayInput, output: &mut ReplayOutput, prefix_len: usize, seat: Seat, observed: &Action) -> bool {
    apply_with(input, output, &mut ReplayState::default(), prefix_len, seat, observed)
}

/// `PreflopStore::query` with one substitution (spec section 8.4): the node history is `branch`'s
/// translated history -- every translated wager replaced by its mapped menu action, resolved back
/// onto the source's own sizes -- while depth, eligibility, rake, roles and the short-handed folds
/// still come from the **observed** prefix (section 8.3 is hindsight-free and money is never
/// rewritten). A branch that translated villain's raise to menu size A therefore looks up the next
/// node under A. Delegates to the chip-only [`PreflopStore::query_history`]: the branch's chip
/// history is all it holds, so a translated edge that two source sizes round to is an explicit
/// unresolved missing path (ruling 13-R1); the walk itself looks nodes up with the source steps it
/// carried, memoized per `(prefix_len, branch.translated, carried steps)`.
pub fn query_translated(store: &PreflopStore, cfg: &HandConfig, state: &HandState, prefix_len: usize, branch: &HistoryBranch) -> PreflopAnswer {
    store.query_history(cfg, state, prefix_len, &branch.translated)
}

/// The public board mask: weight 1 on every combo, 0 on every combo that holds a board card
/// (through Plan 1's `core_ranges::block_public`, never hero's cards).
pub fn board_mask(board: &[Card]) -> Range1326 {
    let mut mask = Range1326([1.0; COMBOS]);
    core_ranges::block_public(&mut mask, board);
    mask
}

/// Spec section 9.2 as amended by revision 6 (S13): the board is applied to each seat's **output
/// marginal** at a street root, never to per-branch masses. For every seat of the list, `m_S` is
/// the maximum of its marginal over the combos the board leaves; every branch's masses for that
/// seat (residual and stopped included) are divided by that one scalar and `ln(m_S)` is added to
/// `log_reach[seat]`. A factor common to a seat's branches changes no posterior and no `q`, so
/// the equal-total invariant and the seat-independent residual share survive, and no per-branch
/// mass is zeroed. A seat with no unblocked support is `InvalidRanges` (the first unsupported
/// reason is kept); its masses are left untouched.
///
/// # Panics
/// Always, through [`marginal`]'s own checks, if a seat has no `log_reach` entry, if a divided mass
/// is not finite or underflows to zero from a positive value, or if the blocked marginal's maximum
/// is not 1 within `MASS_TOLERANCE` after the division.
pub fn block_and_rescale(output: &mut ReplayOutput, board: &[Card]) {
    let mask = board_mask(board);
    let seats: Vec<Seat> = output.branches.first().map(|b| b.seats.iter().map(|s| s.seat).collect()).unwrap_or_default();
    for seat in seats {
        let m = blocked_max(&marginal(&output.branches, seat), &mask);
        if m == 0.0 {
            output.unsupported.get_or_insert(UnsupportedReason::InvalidRanges);
            continue;
        }
        let slot = usize::from(seat.0);
        assert!(slot < output.log_reach.len(), "block_and_rescale: log_reach has {} entries, none for seat {seat:?}", output.log_reach.len());
        for b in output.branches.iter_mut() {
            for s in b.seats.iter_mut().filter(|s| s.seat == seat) {
                for (c, w) in s.mass.iter_mut().enumerate() {
                    let before = *w;
                    *w /= m;
                    assert!(
                        w.is_finite() && (*w > 0.0 || before == 0.0),
                        "block_and_rescale: branch {} seat {seat:?} mass[{c}] = {before} divided by the blocked maximum {m} gives {w}",
                        b.id
                    );
                }
            }
        }
        output.log_reach[slot] += m.ln();
        let peak = blocked_max(&marginal(&output.branches, seat), &mask);
        assert!(
            (peak - 1.0).abs() <= crate::branches::MASS_TOLERANCE && output.log_reach[slot].is_finite(),
            "block_and_rescale: seat {seat:?} rescaled by {m} has blocked maximum {peak} and log_reach {}",
            output.log_reach[slot]
        );
    }
}

/// Publishes every dealt seat's range: its marginal with the current board removed pointwise,
/// narrowed at the output boundary ([`range_output`]); `None` for a vacant seat. `folded_ranges`
/// holds the dealt seats that folded, in increasing seat id; their masses stay in the branches.
/// Hero's cards are never applied (hero-conditioned copies are built after replay, for equity only).
///
/// # Panics
/// Always, through [`range_output`], if a board-free marginal entry exceeds 1: call it after
/// [`block_and_rescale`] with the same board.
pub fn publish(output: &mut ReplayOutput, state: &HandState) {
    let mask = board_mask(&state.board);
    output.ranges = (0..6u8)
        .map(|i| {
            let seat = Seat(i);
            state.dealt.contains(&seat).then(|| public_range(&output.branches, seat, &mask))
        })
        .collect();
    output.folded_ranges =
        (0..6usize).filter(|&i| state.derived.folded[i]).filter_map(|i| output.ranges[i].clone()).collect();
}

/// `seat`'s published range over `branches`: its marginal with the combos `mask` blocks removed
/// pointwise, narrowed at the output boundary ([`range_output`]). The one computation behind
/// [`publish`] and behind the postflop walk's incoming root-range hashes (P3.T15), so a snapshot
/// keyed by the ranges a replay published at a street root is found again by a later replay.
pub(crate) fn public_range(branches: &[HistoryBranch], seat: Seat, mask: &Range1326) -> Range1326 {
    let r = marginal(branches, seat);
    range_output(&r.iter().zip(mask.0.iter()).map(|(w, keep)| if *keep == 0.0 { 0.0 } else { *w }).collect::<Vec<f64>>())
}

// ---------------------------------------------------------------------------------------------
// The transaction.
// ---------------------------------------------------------------------------------------------

/// The per-invocation state of one walk: the mapping memo of spec section 8.3 ("mappings are
/// cached per prefix within a replay run"), keyed by the observed prefix index plus the branch's
/// translated history and carried source steps -- never by final-hand flags -- and the source
/// steps themselves (ruling 13-R1).
#[derive(Debug, Default)]
struct ReplayState {
    lookups: PreflopInvocation,
    /// Per live or stopped branch id: the exact source step chosen at each entry of its
    /// `translated` history -- `Some` at a translated edge, `None` at an observed action the
    /// branch kept as observed. A branch without an entry (a standalone transaction's input)
    /// carries none, so its lookups are chip-only.
    sources: BTreeMap<u8, Vec<Option<PreflopStep>>>,
}

impl ReplayState {
    /// The source steps carried along `b`'s translated history (all `None` when none are).
    ///
    /// # Panics
    /// Always, if a carried list does not have one entry per translated action.
    fn sources_of(&self, b: &HistoryBranch) -> Vec<Option<PreflopStep>> {
        match self.sources.get(&b.id) {
            Some(carried) => {
                assert!(
                    carried.len() == b.translated.len(),
                    "replay: branch {} carries {} source steps for {} translated actions",
                    b.id,
                    carried.len(),
                    b.translated.len()
                );
                carried.clone()
            }
            None => vec![None; b.translated.len()],
        }
    }

    /// The branch's node at `prefix_len`, looked up under its translated history with the source
    /// steps it carries.
    fn lookup(&mut self, input: &ReplayInput, prefix_len: usize, b: &HistoryBranch) -> PreflopAnswer {
        let sources = self.sources_of(b);
        self.lookups.answer_history_sourced(input.store, input.cfg, input.state, prefix_len, &b.translated, &sources)
    }

    /// Keeps the carried steps of `branches` that can still be looked up: every non-residual
    /// branch's (the residual has no history of its own).
    fn retain_for(&mut self, branches: &[HistoryBranch]) {
        self.sources.retain(|id, _| branches.iter().any(|b| b.id == *id && !b.residual));
    }
}

/// What one observed action does to one branch, decided before any branch is replaced: the
/// kernel's [`BranchChoice`] and, for a split, the exact source step of each of its choices, in
/// choice order (empty otherwise).
struct Plan {
    choice: BranchChoice,
    steps: Vec<PreflopStep>,
}

impl Plan {
    fn only(choice: BranchChoice) -> Self {
        Plan { choice, steps: vec![] }
    }
}

fn apply_with(input: &ReplayInput, output: &mut ReplayOutput, run: &mut ReplayState, prefix_len: usize, seat: Seat, observed: &Action) -> bool {
    let recorded = input.state.actions.get(prefix_len);
    assert!(
        recorded.is_some_and(|a| a.street == Street::Preflop && a.seat == seat && a.action == *observed),
        "apply_preflop_action: prefix {prefix_len} records {recorded:?}, not a preflop {observed:?} by seat {}",
        seat.0
    );
    let step = Observed { prefix_len, seat, action: observed };
    let mut notes = Notes::default();
    let plans: Vec<Plan> = output.branches.iter().map(|b| plan_branch(input, run, &step, b, &mut notes)).collect();
    for r in notes.mapping {
        if !output.reasons.contains(&r) {
            output.reasons.push(r);
        }
    }
    output.reasons.extend(notes.events);
    if let Some(reason) = notes.unsupported {
        output.unsupported.get_or_insert(reason);
    }

    // Ruling 13-R3: the complete pre-action generation, each branch with its own choice, in ONE
    // kernel call -- ids allocated and parents remapped once, zero-support removals included.
    let choices: Vec<BranchChoice> = plans.iter().map(|p| p.choice.clone()).collect();
    let batch = split_batch(&output.branches, seat, &choices);

    if batch.applying && !batch.supported {
        // Spec section 9.2: reject the update; every `q` and mass keeps its pre-action value.
        let cause = format!("zero support after {observed:?}");
        let mut carried = BTreeMap::new();
        let kept: Vec<HistoryBranch> = output
            .branches
            .iter()
            .zip(&plans)
            .map(|(b, plan)| {
                let mut sources = run.sources_of(b);
                let mut b = b.clone();
                match &plan.choice {
                    BranchChoice::Keep => {}
                    BranchChoice::Stop(stop) => stop_branch(&mut b, stop.clone()),
                    // The observed on-menu action: the path stays derivable without a guessed action.
                    BranchChoice::Retain { action, .. } => {
                        b.translated.push((seat, *action));
                        sources.push(None);
                    }
                    // Only an unobserved menu size could advance it.
                    BranchChoice::Split(_) => stop_branch(&mut b, cause.clone()),
                }
                carried.insert(b.id, sources);
                b
            })
            .collect();
        output.branches = kept;
        run.sources = carried;
        run.retain_for(&output.branches);
        output.reasons.push(zero_reason(Street::Preflop, seat, observed));
        refresh_nodes(input, output, run, prefix_len + 1);
        return false;
    }

    // Each output branch carries its origin's source steps, plus the step it took here.
    let mut carried = BTreeMap::new();
    for (b, &(i, choice)) in batch.branches.iter().zip(&batch.origin) {
        let mut sources = run.sources_of(&output.branches[i]);
        match (&plans[i].choice, choice) {
            (BranchChoice::Retain { .. }, None) => sources.push(None),
            (BranchChoice::Split(_), Some(c)) => sources.push(Some(plans[i].steps[c].clone())),
            _ => {}
        }
        carried.insert(b.id, sources);
    }
    run.sources = carried;
    output.branches = batch.branches;
    if batch.supported {
        cap_branches(&mut output.branches); // one global cap per observed action
        rescale(&mut output.branches, &mut output.log_reach);
    }
    run.retain_for(&output.branches);
    refresh_nodes(input, output, run, prefix_len + 1);
    true
}

/// The observed action being applied: the `prefix_len`-th recorded action, by `seat`.
struct Observed<'a> {
    prefix_len: usize,
    seat: Seat,
    action: &'a Action,
}

/// What one transaction records besides its plans: the lookups' mapping reasons (deduplicated
/// against the output when merged), each branch's own events (stops, translations), and the
/// first unsupported answer that is not a coverage gap.
#[derive(Default)]
struct Notes {
    mapping: Vec<ApproxReason>,
    events: Vec<ApproxReason>,
    unsupported: Option<UnsupportedReason>,
}

/// Decides what the observed action does to branch `b` (see [`Plan`]).
fn plan_branch(input: &ReplayInput, run: &mut ReplayState, step: &Observed, b: &HistoryBranch, notes: &mut Notes) -> Plan {
    if b.residual || b.stopped.is_some() {
        return Plan::only(BranchChoice::Keep);
    }
    let (prefix_len, seat, observed) = (step.prefix_len, step.seat, step.action);
    assert!(
        b.translated.len() == prefix_len,
        "apply_preflop_action: live branch {} carries {} translated actions at prefix {prefix_len}; a live branch advances once per observed preflop action",
        b.id,
        b.translated.len()
    );
    let answer = run.lookup(input, prefix_len, b);
    for r in &answer.reasons {
        if !notes.mapping.contains(r) {
            notes.mapping.push(r.clone());
        }
    }
    let (Some(node), Some(expanded), Some(key)) = (answer.node.as_ref(), answer.expanded.as_ref(), answer.source_key.as_ref()) else {
        return Plan::only(stop_without_node(seat, &answer, notes));
    };
    assert_eq!(answer.actor, Some(seat), "apply_preflop_action: the prefix's actor is the observed actor");
    if let Some(a) = menu_step_index(&node.actions, observed, answer.unit) {
        // On the menu: one `condition` with that action's column. The branch records the OBSERVED
        // action (spec section 8.4 replaces only translated wagers; ruling 13-R1), whose own
        // resolution against this node is the step `a` it was conditioned with.
        return Plan::only(BranchChoice::Retain { action: *observed, p: likelihood(expanded, a) });
    }
    let Some(to) = raise_to(observed) else {
        // A non-wager this node does not list has no likelihood here, and none is guessed.
        let cause = format!("unmappable action {observed:?} at {}", answer.key);
        notes.events.push(stop_reason(seat, &cause));
        return Plan::only(BranchChoice::Stop(cause));
    };
    // Ruling 13-R2: the pot fractions at the SOURCE parent, in one exact scale.
    let parent = source_parent(key, node.actor, answer.unit);
    let mapped = source_fraction(u64::from(to) * 1000, &parent)
        .zip(source_menu(node, expanded, &parent, answer.unit))
        .and_then(|(s, menu)| interpolate(s, &menu).map(|t| (s, menu, t)));
    let Some((s, menu, t)) = mapped else {
        let cause = format!("unmappable size at {}", answer.key);
        notes.events.push(stop_reason(seat, &cause));
        return Plan::only(BranchChoice::Stop(cause));
    };
    notes.events.push(translation_reason(Street::Preflop, seat, s, &menu, &t));
    Plan {
        choice: BranchChoice::Split(t.choices.iter().map(|&(a, f)| (expanded.actions[a], f, likelihood(expanded, a))).collect()),
        steps: t.choices.iter().map(|&(a, _)| node.actions[a].clone()).collect(),
    }
}

/// The stop of a branch whose lookup found no node for the actor: `missing node <key>` for a
/// coverage gap (spec section 9.3) -- an absent node, an off-menu history, or a translated edge a
/// chip-only lookup cannot resolve; any other unsupported answer (a format, history or config the
/// store cannot map at all) also marks the replay `unsupported`, keeping the first reason.
fn stop_without_node(seat: Seat, answer: &PreflopAnswer, notes: &mut Notes) -> BranchChoice {
    match answer.unsupported.as_ref() {
        Some(UnsupportedReason::MissingPreflopNode { key }) => {
            notes.events.push(missing_reason(seat, key));
            BranchChoice::Stop(format!("missing node {key}"))
        }
        Some(other) => {
            notes.unsupported.get_or_insert_with(|| other.clone());
            let cause = format!("unsupported preflop lookup: {other:?}");
            notes.events.push(stop_reason(seat, &cause));
            BranchChoice::Stop(cause)
        }
        None => panic!("apply_preflop_action: a preflop answer without a node states why: {answer:?}"),
    }
}

/// `UnconditionedPriorStreet{Preflop, seat, cause}`, the cause being the branch's stop cause.
fn stop_reason(seat: Seat, cause: &str) -> ApproxReason {
    ApproxReason::UnconditionedPriorStreet { street: Street::Preflop, seat, cause: cause.to_string() }
}

/// One menu action's likelihood column over 1326 combos, as `f64`: `P(a | c)` from the expanded
/// node, and 0 for a combo of an explicitly unreachable class (the source's own statement that the
/// class never reaches this node; a zero-mass class stays zero and no likelihood is invented).
fn likelihood(node: &ExpandedNode, a: usize) -> Vec<f64> {
    assert!(
        node.probs.len() == COMBOS && node.available.len() == COMBOS,
        "likelihood: an expanded node has {} probability rows and {} availability flags, expected {COMBOS}",
        node.probs.len(),
        node.available.len()
    );
    node.probs.iter().zip(&node.available).map(|(row, &available)| if available { f64::from(row[a]) } else { 0.0 }).collect()
}

/// The chip amount of a wager (`Bet`, `Raise`, `AllIn`), `None` for fold, check and call.
fn raise_to(action: &Action) -> Option<u32> {
    match action {
        Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => Some(*to),
        Action::Fold | Action::Check | Action::Call => None,
    }
}

/// The money at a mapped source parent (spec section 8.4, ruling 13-R2), in **milli-chips at the
/// source's own amounts**: a source amount of `x` thousandths of a source unit is `x * unit`
/// (exact), and an actual chip amount `c` is `c * 1000` (exact). A fractional source amount (2.5
/// bb at a 3-chip unit is 7.5 chips) and an observed chip amount therefore share one integer scale,
/// and no amount is rounded to the chip before a pot fraction is taken. `own` is the actor's
/// contribution, `call` what it owes, `pot` the pot before its call, all in that scale.
struct SourceParent {
    own: u64,
    call: u64,
    pot: u64,
}

/// A slot per position for [`source_parent`]'s contributions.
fn position_slot(p: Position) -> usize {
    match p {
        Position::Btn => 0,
        Position::Sb => 1,
        Position::Bb => 2,
        Position::Utg => 3,
        Position::Hj => 4,
        Position::Co => 5,
    }
}

/// The source parent of `actor`'s decision at the node found under `key` (spec section 8.4:
/// "reconstructed from the financial state at the mapped parent node"), rebuilt from the source's
/// own tree in source units: the source posts (0.5 and 1 source unit, spec section 8.2's fixed
/// source blinds, by the key's -- possibly virtual -- roles: behind a straddle the virtual SB and
/// BB post and the physical SB's post is not represented, spec section 8.3), then every step of
/// the key's history -- fold and check add nothing, a call matches the highest contribution, a
/// raise sets the contribution to its exact source size, an all-in to the source stack
/// (`depth_bb`) -- each capped at that source stack. Never the live posts, the live stacks, or a
/// chip-rounded size.
fn source_parent(key: &PreflopNodeKey, actor: Position, unit: u32) -> SourceParent {
    let unit = u64::from(unit);
    let stack = u64::from(key.depth_bb) * 1000 * unit;
    let mut committed = [0_u64; 6];
    committed[position_slot(Position::Sb)] = 500 * unit;
    committed[position_slot(Position::Bb)] = 1000 * unit;
    for (position, step) in &key.history {
        let highest = committed.iter().copied().max().unwrap_or(0);
        let to = match step {
            PreflopStep::Fold | PreflopStep::Check => continue,
            PreflopStep::Call => highest,
            PreflopStep::Raise { to_bb_x1000 } => u64::from(*to_bb_x1000) * unit,
            PreflopStep::AllIn => stack,
        };
        let slot = &mut committed[position_slot(*position)];
        *slot = (*slot).max(to.min(stack));
    }
    let own = committed[position_slot(actor)];
    let highest = committed.iter().copied().max().unwrap_or(0);
    SourceParent { own, call: highest.min(stack).saturating_sub(own), pot: committed.iter().sum() }
}

/// Spec section 8.4's pot fraction of a wager to `to` (milli-chips) at `parent`:
/// `(to - own - call) / (pot + call)`, computed from exact integers -- the wide, one-scale twin of
/// `core_preflop::wager_fraction`, with the same domain. `None` when there is nothing to take a
/// fraction of (`pot + call == 0`) or the amount does not cover the call (`to < own + call`): a
/// recoverable translation-domain mismatch (the observed amount against a mapped parent whose
/// call follows a larger translated size), never a panic.
fn source_fraction(to: u64, parent: &SourceParent) -> Option<f64> {
    let covered = parent.own + parent.call;
    let denom = parent.pot + parent.call;
    if denom == 0 || to < covered {
        return None;
    }
    let s = (to - covered) as f64 / denom as f64;
    s.is_finite().then_some(s)
}

/// The node's own wager sizes as the `(menu index, pot fraction)` pairs `interpolate` takes, at
/// `parent` (spec section 8.4: "the source node's menu fractions"): a `Raise` at its exact source
/// size (`to_bb_x1000 * unit` milli-chips, never its chip rounding) and `AllIn` at the actor's
/// ACTUAL maximum, the expanded node's chip amount (P3.T9), times 1000. `None` when any size has
/// no fraction there, so a bracket is never silently narrowed (as `core_preflop::menu_fractions`).
///
/// # Panics
/// Always, if the expanded node's action at a source `AllIn` is not an `AllIn`.
fn source_menu(node: &PreflopNode, expanded: &ExpandedNode, parent: &SourceParent, unit: u32) -> Option<Vec<(usize, f64)>> {
    node.actions
        .iter()
        .enumerate()
        .filter_map(|(i, step)| {
            let to = match step {
                PreflopStep::Raise { to_bb_x1000 } => u64::from(*to_bb_x1000) * u64::from(unit),
                PreflopStep::AllIn => match expanded.actions[i] {
                    Action::AllIn { to } => u64::from(to) * 1000,
                    other => panic!("source_menu: the source all-in at menu index {i} expanded to {other:?}"),
                },
                PreflopStep::Fold | PreflopStep::Check | PreflopStep::Call => return None,
            };
            Some(source_fraction(to, parent).map(|s| (i, s)))
        })
        .collect()
}

/// Spec section 8.4's disclosure of one translated wager: the observed pot fraction, the mapped
/// menu sizes (as pot fractions at the mapped parent: the source parent preflop, the snapshot
/// node's mapped financial prefix postflop) with their interpolation weights, the deviation, and
/// `prominent = d > 0.10`.
pub(crate) fn translation_reason(street: Street, seat: Seat, s: f64, menu: &[(usize, f64)], t: &Interpolation) -> ApproxReason {
    let size = |i: usize| menu.iter().find(|(j, _)| *j == i).map(|(_, x)| *x).expect("an interpolation choice is a menu size");
    ApproxReason::BetTranslation {
        street,
        seat,
        observed_pct: s as f32,
        mapped: t.choices.iter().map(|&(i, f)| (size(i) as f32, f as f32)).collect(),
        deviation: t.deviation as f32,
        prominent: t.deviation > 0.10,
    }
}

/// Sets each live branch's node for the seat to act at `prefix` (the key its next lookup will use,
/// if the source has that node) and clears every other seat's node; a residual or stopped branch
/// has no node for any seat.
fn refresh_nodes(input: &ReplayInput, output: &mut ReplayOutput, run: &mut ReplayState, prefix: usize) {
    for b in output.branches.iter_mut() {
        for s in &mut b.seats {
            s.node = None;
        }
        if b.residual || b.stopped.is_some() || b.translated.len() != prefix {
            continue;
        }
        let answer = run.lookup(input, prefix, b);
        if let (Some(actor), Some(_)) = (answer.actor, answer.node.as_ref()) {
            if let Some(s) = b.seats.iter_mut().find(|s| s.seat == actor) {
                s.node = Some(answer.key);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Street roots.
// ---------------------------------------------------------------------------------------------

/// The street whose root the replay ends at: the street of the board on record (0, 3, 4 or 5
/// cards), which `core_model` guarantees is the one the history reached.
///
/// # Panics
/// Always, on a board of any other length.
fn current_street(state: &HandState) -> Street {
    match state.board.len() {
        0 => Street::Preflop,
        3 => Street::Flop,
        4 => Street::Turn,
        5 => Street::River,
        n => panic!("replay: a {n}-card board is not a street board (0, 3, 4 or 5 cards)"),
    }
}

/// The postflop streets completed before `current`, in order: none on the preflop or the flop,
/// the flop on the turn, the flop then the turn on the river.
fn completed_streets(current: Street) -> Vec<Street> {
    [Street::Flop, Street::Turn, Street::River].into_iter().filter(|s| *s < current).collect()
}

/// A street's root board: the first `board_len` cards of the board on record.
fn root_board(state: &HandState, street: Street) -> &[Card] {
    &state.board[..street.board_len()]
}

/// Entering a postflop street ends every preflop stop (spec section 9.3 scopes it to the preflop
/// street); the residual is never touched.
fn clear_preflop_stops(output: &mut ReplayOutput) {
    for b in output.branches.iter_mut().filter(|b| !b.residual) {
        b.stopped = None;
    }
}

/// The maximum of `r` over the combos `mask` keeps.
fn blocked_max(r: &[f64], mask: &Range1326) -> f64 {
    r.iter().zip(mask.0.iter()).map(|(w, keep)| if *keep == 0.0 { 0.0 } else { *w }).fold(0.0_f64, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The residual never clears and never becomes live on entering a postflop street; a
    /// stopped live branch does.
    #[test]
    fn entering_postflop_clears_stops_but_never_the_residual() {
        let seats = [Seat(0), Seat(1), Seat(5)];
        let mut stopped = initial(&seats).remove(0);
        stopped.id = 1;
        stopped.q = 0.25;
        stop_branch(&mut stopped, "missing node x".into());
        let mut residual = initial(&seats).remove(0);
        residual.id = 2;
        residual.q = 0.5;
        residual.residual = true;
        let mut output = ReplayOutput {
            ranges: vec![None; 6],
            branches: vec![stopped, residual],
            folded_ranges: vec![],
            log_reach: vec![0.0; 6],
            reasons: vec![],
            unsupported: None,
        };
        clear_preflop_stops(&mut output);
        assert_eq!(output.branches[0].stopped, None);
        assert!(!output.branches[0].residual);
        assert!(output.branches[1].residual, "the residual stays the residual");
        assert_eq!(output.branches[1].stopped, None);
        assert_eq!((output.branches[0].q, output.branches[1].q), (0.25, 0.5), "no weight changes");
    }

    /// Completed streets are the postflop streets strictly before the current root, in order.
    #[test]
    fn completed_streets_are_the_postflop_streets_before_the_current_one() {
        assert_eq!(completed_streets(Street::Preflop), vec![]);
        assert_eq!(completed_streets(Street::Flop), vec![]);
        assert_eq!(completed_streets(Street::Turn), vec![Street::Flop]);
        assert_eq!(completed_streets(Street::River), vec![Street::Flop, Street::Turn]);
    }
}
