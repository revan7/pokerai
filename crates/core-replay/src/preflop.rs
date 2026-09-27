//! Preflop replay with frozen missing-node stops (spec sections 8.3, 8.4, 9.1-9.3; plan 3 Task 13).
//!
//! [`replay`] starts from one branch with `q = 1` and uniform masses for every dealt seat, applies
//! every observed preflop action once in every live branch ([`walk_preflop`],
//! [`apply_preflop_action`]), falls back to unconditioned completed postflop streets (Task 15
//! walks them through snapshots), blocks the board on each street root's output marginal
//! ([`block_and_rescale`]) and publishes every dealt seat's range ([`publish`]).
//!
//! # One observed preflop action is one transaction
//!
//! For every branch of the shared list, in order:
//!
//! - a residual or stopped branch is copied through unchanged (frozen);
//! - a live branch looks up the actor's node under **its own translated history**
//!   ([`query_translated`]; the observed prefix still supplies depth, eligibility, rake, roles and
//!   the short-handed folds). No node (spec section 9.3): the branch stops for the rest of the
//!   preflop street -- `q` and every mass frozen, no node for any seat, reason
//!   `UnconditionedPriorStreet{Preflop, seat, "missing node <key>"}` -- and is never resumed at a
//!   later present node;
//! - the observed action on the node's menu (section 8.3's size rule): one [`condition`] with
//!   that action's column, the child keeping the branch's id;
//! - an off-menu wager: section 8.4's pseudo-harmonic interpolation at the branch's mapped parent
//!   (`wager_fraction`/`menu_fractions`/`interpolate`, Task 10), disclosed as `BetTranslation`,
//!   and the branch is split over the mapped menu sizes by the kernel's [`split_action`]; a size
//!   with no pot fraction there stops the branch (`unmappable size at <key>`).
//!
//! Every applying branch's candidates are built before anything is decided. If no applying branch
//! has support (spec section 9.2's zero-support rule), the update is rejected: every `q` and mass
//! keeps its pre-action value and `UnconditionedPriorStreet{"zero support after <action>"}` is
//! added. Navigation is still kept truthful without a guessed action: a branch whose observed
//! action was on its menu advances along that one mapped action (no likelihood), and a branch
//! whose action was an off-menu wager -- which could only advance along an unobserved menu size --
//! stops. Otherwise the candidates replace the list, the cap runs once ([`cap_branches`]) and
//! every seat is rescaled once ([`rescale`]).
//!
//! # Ruling 13-pre and the kernel's split
//!
//! A generation is expanded through [`split_action`] over the **whole** branch list, so ids are
//! allocated after every id of the generation and parent links come from the kernel's own
//! compaction pass (never per-branch slices, whose ids collide). `split_action` applies one common
//! choice list to every live branch it is given, while replay branches each carry their own menu;
//! every other live branch is therefore held frozen for the call and released after it. When a
//! single branch splits at an observed action -- every split the synthetic source can produce --
//! that is exactly one call per observed action. When several branches split at the same action
//! with different menus, one whole-list call is made per splitting branch, each over the list the
//! previous call returned, in list order.

use crate::branches::{
    cap_branches, condition, initial, marginal, missing_reason, range_output, rescale, split_action, stop_branch, zero_reason,
    HistoryBranch,
};
use crate::snapshot::StreetSnapshot;
use core_preflop::{
    interpolate, menu_fractions, menu_step_index, prefix_state, wager_fraction, ExpandedNode, Interpolation, PreflopAnswer,
    PreflopInvocation, PreflopStore,
};
use proto::{Action, ApproxReason, Card, HandConfig, HandState, Range1326, Seat, Street, UnsupportedReason, COMBOS};

/// Everything one replay reads (spec section 9.1). `snapshots` are the street solutions
/// registered for this hand; Task 13 consumes none of them (selection is Task 14, the postflop
/// walk Task 15).
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
/// blocked ([`block_and_rescale`]) and, until Task 15 consumes snapshots, left unconditioned with
/// `UnconditionedPriorStreet{street, seat, "no compatible snapshot"}` for every seat that acted on
/// it -- and finally the current street's root blocked and every range published ([`publish`]).
/// The current street's own postflop actions are never replayed: Plan 2's street-root solve inserts
/// them exactly. A preflop stop is scoped to the preflop street (spec section 9.3), so it clears
/// once the hand has entered a postflop street; the residual never changes.
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
        unconditioned_street(&input, street, &mut output);
    }
    block_and_rescale(&mut output, root_board(input.state, current));
    publish(&mut output, input.state);
    output
}

/// Applies every observed preflop action of `input.state`, in order, through
/// [`apply_preflop_action`]'s transaction, stopping at the first postflop action. One mapping memo
/// ([`PreflopInvocation`]) serves the whole walk, keyed by the observed prefix index and each
/// branch's translated history. `output` must be the start state [`replay`] builds (every live
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
/// # Panics
/// Always, if `(seat, observed)` is not the preflop action recorded at `prefix_len`, if a live
/// branch does not carry exactly `prefix_len` translated actions, or through the kernel's own
/// invariant checks ([`condition`], [`split_action`], [`cap_branches`], [`rescale`]).
pub fn apply_preflop_action(input: &ReplayInput, output: &mut ReplayOutput, prefix_len: usize, seat: Seat, observed: &Action) -> bool {
    apply_with(input, output, &mut ReplayState::default(), prefix_len, seat, observed)
}

/// `PreflopStore::query` with one substitution (spec section 8.4): the node history is `branch`'s
/// translated history -- every translated wager replaced by its mapped menu action, resolved back
/// onto the source's own sizes -- while depth, eligibility, rake, roles and the short-handed folds
/// still come from the **observed** prefix (section 8.3 is hindsight-free and money is never
/// rewritten). A branch that translated villain's raise to menu size A therefore looks up the next
/// node under A. Delegates to [`PreflopStore::query_history`]; the walk memoizes it per
/// `(prefix_len, branch.translated)`.
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
            state.dealt.contains(&seat).then(|| {
                let r = marginal(&output.branches, seat);
                range_output(&r.iter().zip(mask.0.iter()).map(|(w, keep)| if *keep == 0.0 { 0.0 } else { *w }).collect::<Vec<f64>>())
            })
        })
        .collect();
    output.folded_ranges =
        (0..6usize).filter(|&i| state.derived.folded[i]).filter_map(|i| output.ranges[i].clone()).collect();
}

// ---------------------------------------------------------------------------------------------
// The transaction.
// ---------------------------------------------------------------------------------------------

/// The per-invocation state of one walk: the mapping memo of spec section 8.3 ("mappings are
/// cached per prefix within a replay run"), keyed by the observed prefix index plus the branch's
/// translated history -- never by final-hand flags.
#[derive(Debug, Default)]
struct ReplayState {
    lookups: PreflopInvocation,
}

impl ReplayState {
    fn lookup(&mut self, input: &ReplayInput, prefix_len: usize, branch: &HistoryBranch) -> PreflopAnswer {
        self.lookups.answer_history(input.store, input.cfg, input.state, prefix_len, &branch.translated)
    }
}

/// What one observed action does to one branch, decided before any branch is replaced.
enum Plan {
    /// A residual or already stopped branch: copied through unchanged.
    Frozen,
    /// The actor has no node here, or the action cannot be mapped: the branch stops, frozen.
    Stop(String),
    /// On the menu: the mapped menu action and the conditioned child (`None` when `M_k = 0`).
    OnMenu { action: Action, child: Option<HistoryBranch> },
    /// An off-menu wager: this branch's own interpolation choices for `split_action`.
    Split { choices: Vec<(Action, f64, Vec<f64>)> },
}

/// A stopped branch's cause marker while another branch of the same generation is split: it can
/// never be a genuine cause (it opens with a NUL), and it never outlives [`split_one`].
const HELD: &str = "\u{0}held while another branch of this generation is split";

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
    let applying = plans.iter().any(|p| matches!(p, Plan::OnMenu { .. } | Plan::Split { .. }));

    // Every applying branch's candidates, in list order, before anything is decided.
    let mut next: Vec<HistoryBranch> = Vec::with_capacity(output.branches.len() + 1);
    let mut targets: Vec<(usize, &[(Action, f64, Vec<f64>)])> = Vec::new();
    let mut supported = false;
    for (b, plan) in output.branches.iter().zip(&plans) {
        match plan {
            Plan::Frozen => next.push(b.clone()),
            Plan::Stop(cause) => {
                let mut stopped = b.clone();
                stop_branch(&mut stopped, cause.clone());
                next.push(stopped);
            }
            Plan::OnMenu { action, child } => {
                if let Some(child) = child {
                    let mut child = child.clone();
                    child.translated.push((seat, *action));
                    supported = true;
                    next.push(child);
                }
            }
            Plan::Split { choices } => {
                targets.push((next.len(), choices.as_slice()));
                next.push(b.clone());
            }
        }
    }
    let mut offset: isize = 0;
    for (position, choices) in targets {
        let at = usize::try_from(position as isize + offset).expect("a split target stays in the list");
        let before = next.len();
        next = split_one(next, at, seat, choices);
        let children = next.len() + 1 - before;
        supported |= children > 0;
        offset += children as isize - 1;
    }

    if applying && !supported {
        // Spec section 9.2: reject the update; every `q` and mass keeps its pre-action value.
        let cause = format!("zero support after {observed:?}");
        let kept: Vec<HistoryBranch> = output
            .branches
            .iter()
            .zip(&plans)
            .map(|(b, plan)| {
                let mut b = b.clone();
                match plan {
                    Plan::Frozen => {}
                    Plan::Stop(stop) => stop_branch(&mut b, stop.clone()),
                    // The one mapped action: the path stays derivable without a guessed action.
                    Plan::OnMenu { action, .. } => b.translated.push((seat, *action)),
                    // Only an unobserved menu size could advance it.
                    Plan::Split { .. } => stop_branch(&mut b, cause.clone()),
                }
                b
            })
            .collect();
        output.branches = kept;
        output.reasons.push(zero_reason(Street::Preflop, seat, observed));
        refresh_nodes(input, output, run, prefix_len + 1);
        return false;
    }
    output.branches = next;
    if supported {
        cap_branches(&mut output.branches); // one global cap per observed action
        rescale(&mut output.branches, &mut output.log_reach);
    }
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
        return Plan::Frozen;
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
    let (Some(node), Some(expanded)) = (answer.node.as_ref(), answer.expanded.as_ref()) else {
        return stop_without_node(seat, &answer, notes);
    };
    assert_eq!(answer.actor, Some(seat), "apply_preflop_action: the prefix's actor is the observed actor");
    if let Some(a) = menu_step_index(&node.actions, observed, answer.unit) {
        let p = likelihood(expanded, a);
        return Plan::OnMenu { action: expanded.actions[a], child: condition(b, seat, &p, 1.0) };
    }
    let Some(to) = raise_to(observed) else {
        // A non-wager this node does not list has no likelihood here, and none is guessed.
        let cause = format!("unmappable action {observed:?} at {}", answer.key);
        notes.events.push(stop_reason(seat, &cause));
        return Plan::Stop(cause);
    };
    let (own, call, pot) = parent_money(input, b, seat);
    let mapped = wager_fraction(to, own, call, pot)
        .zip(menu_fractions(&expanded.actions, own, call, pot))
        .and_then(|(s, menu)| interpolate(s, &menu).map(|t| (s, menu, t)));
    let Some((s, menu, t)) = mapped else {
        let cause = format!("unmappable size at {}", answer.key);
        notes.events.push(stop_reason(seat, &cause));
        return Plan::Stop(cause);
    };
    notes.events.push(translation_reason(Street::Preflop, seat, s, &menu, &t));
    Plan::Split { choices: t.choices.iter().map(|&(a, f)| (expanded.actions[a], f, likelihood(expanded, a))).collect() }
}

/// The stop of a branch whose lookup found no node for the actor: `missing node <key>` for a
/// coverage gap (spec section 9.3); any other unsupported answer (a format, history or config the
/// store cannot map at all) also marks the replay `unsupported`, keeping the first reason.
fn stop_without_node(seat: Seat, answer: &PreflopAnswer, notes: &mut Notes) -> Plan {
    match answer.unsupported.as_ref() {
        Some(UnsupportedReason::MissingPreflopNode { key }) => {
            notes.events.push(missing_reason(seat, key));
            Plan::Stop(format!("missing node {key}"))
        }
        Some(other) => {
            notes.unsupported.get_or_insert_with(|| other.clone());
            let cause = format!("unsupported preflop lookup: {other:?}");
            notes.events.push(stop_reason(seat, &cause));
            Plan::Stop(cause)
        }
        None => panic!("apply_preflop_action: a preflop answer without a node states why: {answer:?}"),
    }
}

/// `UnconditionedPriorStreet{Preflop, seat, cause}`, the cause being the branch's stop cause.
fn stop_reason(seat: Seat, cause: &str) -> ApproxReason {
    ApproxReason::UnconditionedPriorStreet { street: Street::Preflop, seat, cause: cause.to_string() }
}

/// Expands the branch at `at` through one [`split_action`] call over the whole generation, every
/// other live branch held frozen for the call (see the module docs, ruling 13-pre): ids come after
/// every id of the generation and parents from the kernel's compaction pass.
///
/// # Panics
/// Always, if the target is not a live branch or some branch already carries the hold marker.
fn split_one(mut generation: Vec<HistoryBranch>, at: usize, seat: Seat, choices: &[(Action, f64, Vec<f64>)]) -> Vec<HistoryBranch> {
    assert!(
        !generation[at].residual && generation[at].stopped.is_none(),
        "split_one: branch {} is not live",
        generation[at].id
    );
    for (i, b) in generation.iter_mut().enumerate() {
        assert!(b.stopped.as_deref() != Some(HELD), "split_one: branch {} is already held", b.id);
        if i != at && !b.residual && b.stopped.is_none() {
            b.stopped = Some(HELD.to_string());
        }
    }
    let mut out = split_action(&generation, seat, choices);
    for b in &mut out {
        if b.stopped.as_deref() == Some(HELD) {
            b.stopped = None;
        }
    }
    out
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

/// `(own, call, pot)` in chips at the mapped parent of `seat`'s next action in `branch`: the live
/// posts (the model's own, at prefix 0), then every action of the branch's translated history --
/// fold and check add nothing, a call matches the highest contribution, a wager sets the
/// contribution to its menu amount -- each capped at the seat's starting stack. This is the
/// branch's source path expressed in the chip domain that `wager_fraction` and `menu_fractions`
/// (and the expanded menu they read) use; it is a standalone replay, never
/// `core_model::apply_action`, so a rounded source raise that would be illegal at the live table
/// never touches the observed hand (source navigation and model legality are separate).
fn parent_money(input: &ReplayInput, branch: &HistoryBranch, seat: Seat) -> (u32, u32, u32) {
    let root = prefix_state(input.state, 0);
    let mut committed: [u64; 6] = std::array::from_fn(|i| u64::from(root.derived.committed_this_street[i]));
    let cap: [u64; 6] = std::array::from_fn(|i| committed[i] + u64::from(root.derived.stacks_remaining[i]));
    for (s, a) in &branch.translated {
        let i = usize::from(s.0);
        let highest = committed.iter().copied().max().unwrap_or(0);
        committed[i] = match a {
            Action::Fold | Action::Check => committed[i],
            Action::Call => highest.min(cap[i]).max(committed[i]),
            Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => u64::from(*to).min(cap[i]).max(committed[i]),
        };
    }
    let i = usize::from(seat.0);
    let highest = committed.iter().copied().max().unwrap_or(0);
    let own = committed[i];
    let call = highest.min(cap[i]).saturating_sub(own);
    let pot: u64 = committed.iter().sum();
    let narrow = |x: u64, what: &str| u32::try_from(x).unwrap_or_else(|_| panic!("parent_money: {what} {x} exceeds the chip domain"));
    (narrow(own, "own contribution"), narrow(call, "call"), narrow(pot, "pot"))
}

/// Spec section 8.4's disclosure of one translated wager: the observed pot fraction, the mapped
/// menu sizes (as pot fractions at the mapped parent) with their interpolation weights, the
/// deviation, and `prominent = d > 0.10`.
fn translation_reason(street: Street, seat: Seat, s: f64, menu: &[(usize, f64)], t: &Interpolation) -> ApproxReason {
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

/// Task 13's fallback for a completed postflop street (no snapshot is consumed yet): the masses
/// are kept exactly as the earlier streets left them, and every seat that acted on it gets
/// `UnconditionedPriorStreet{street, seat, "no compatible snapshot"}` once, in order of its first
/// action (spec section 9.3).
fn unconditioned_street(input: &ReplayInput, street: Street, output: &mut ReplayOutput) {
    let mut acted: Vec<Seat> = Vec::new();
    for a in input.state.actions.iter().filter(|a| a.street == street) {
        if !acted.contains(&a.seat) {
            acted.push(a.seat);
        }
    }
    for seat in acted {
        output.reasons.push(ApproxReason::UnconditionedPriorStreet { street, seat, cause: "no compatible snapshot".into() });
    }
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
