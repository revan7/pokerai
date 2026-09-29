//! Spec 6's `experimental` block (plan 4 Task 11): beside a multiway decision's `Unsupported{MultiwayEv}` result, and
//! never inside it, an isolated synthetic-root heads-up solve with a contract separate from every main-path solve.
//!
//! The surrogate (spec 6, "`experimental` (multiway only; outline §5)"):
//! - the opponent is the seat whose street-root public range has the highest range-vs-range equity against hero's
//!   public range (`select_opponent` / `choose_opponent`, over plan 2 Task 25's `equity::range_vs_range`, the one
//!   equity routine), chosen inside one absolute cutoff of the equity phase on the engine clock (fix round 1,
//!   P4T11-I2); a selection that cannot finish inside it, or finds no opponent, names its cause (`NoOpponent`);
//! - the synthetic root (`surrogate_input`) takes the current total pot at the decision (every chip committed, on every
//!   street), both stacks the smaller of hero's and that opponent's remaining stacks, an **empty history**, and hero OOP
//!   iff hero precedes the opponent in postflop order (`core_model::postflop_order`); it is skipped when the opponent is
//!   all-in or the stack or the pot is 0;
//! - the ranges are hero's and the opponent's public ranges at the current street root (the current street's actions
//!   not applied: "unconditioned ranges", stated in `ranges_used`), never hero-conditioned;
//! - the street's own template and what is left of the street budget; hero's advice is read, for hero's actual combo
//!   (the only use of hero's cards), at the synthetic root when hero is OOP and at the node after OOP's check when IP,
//!   with `ev_bb = ev_chips / bb_chips` over the hand's real big blind (a fold exactly 0).
//!
//! Isolation. The surrogate never builds a `SolveInput`: it builds its worker request from its parts
//! (`solve::solve_request_from_parts`) and sends it through the solve client's shared transport
//! (`solve::send_solve_request`: one attempt, whole-solution validation, identity and expiry at every sliced receive,
//! heartbeat, cancel). It never reads or writes the cache, never registers a snapshot or records a miss, and never
//! touches the main result's assumptions. It runs inside the live request's own street deadline and `Final` claim and
//! arms nothing: no second deadline arithmetic, no second worker link, no retry, no restart (a worker that failed is
//! killed; the next solve relaunches it). Whatever keeps it from answering is a `Skipped` whose reason the multiway
//! `Final` names in a note (`absent_note`); that `Final` is otherwise exactly what it was without the surrogate.

use crate::clock::{Clock, SystemClock};
use crate::core::EngineCore;
use crate::deadline::Deadlines;
use crate::ranges::jointly_compatible;
use crate::replay_bridge::miss_cause;
use crate::solve::{send_solve_request, solve_request_from_parts, worker_for_request, SolvePlan, Terminal};
use crate::tree::{build_tree_full, TemplateSelection};
use crate::watchdog::{SharedSink, StreetDeadline};
use proto::{
    combo_index, resolve_chip_path, Action, ActionAdvice, Card, DecisionIdentity, Derived, EquityMethod, ExperimentalHu, HandState, Rake, Range1326, Seat, Street,
    StreetRootSnapshot, EXPERIMENTAL_NOTE,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The prefix of the note a multiway `Final` carries when its `experimental` block is absent (`absent_note`).
pub const ABSENT_NOTE: &str = "experimental block absent: ";

/// The note of a multiway `Final` whose `experimental` block is absent, naming `why`.
pub fn absent_note(why: &str) -> String {
    format!("{ABSENT_NOTE}{why}")
}

/// The synthetic heads-up root of spec 6: hero, the chosen opponent, the total pot, the stack both sides are given
/// (the smaller remaining stack), hero's role (`"oop"` or `"ip"`), the street's template, and how many seats were in
/// the pot (the synthetic root's `projected_from`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurrogateInput {
    pub hero: Seat,
    pub opponent: Seat,
    pub pot: u32,
    pub stack: u32,
    pub hero_role: &'static str,
    pub template_id: String,
    pub pot_eligible: u8,
}

/// The pairwise estimate opponent selection runs: the first range's equity against the second's on `board`, within
/// the allowance given, polling the cancellation token. Production is `equity::range_vs_range` (spec 6: the same equity
/// routine as spec 4.4's range-vs-range population), the only evaluator; a test may replace it
/// (`serve::ServeSeams::opponent_equity`).
pub type PairEquity = Arc<dyn Fn(&Range1326, &Range1326, &[Card], Duration, &AtomicBool) -> Option<(f32, EquityMethod)> + Send + Sync>;

/// Why opponent selection chose no seat (fix round 1, P4T11-M1): each cause is named as it is, and only the cutoff is an
/// overrun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoOpponent {
    /// No seat but hero was offered.
    NoCandidates,
    /// The request's equity token was set (the decision superseded, ruling 28-I4).
    Cancelled,
    /// The equity phase's cutoff came before an opponent was chosen: an estimate was never started, or the selection
    /// finished after it (spec 6: skipped when the equity phase overran).
    Overran,
    /// No other seat's public range holds a combo compatible with one of hero's: no pairwise-compatible equity exists.
    Incompatible,
    /// Every estimate started ended inside the phase without a value (its allowance ran out, or its pair cannot reach a
    /// showdown on this board): the pair's cause is not told apart.
    NotComputed,
}

impl NoOpponent {
    /// The reason a multiway `Final`'s note gives for the absent block.
    pub fn why(self) -> &'static str {
        match self {
            NoOpponent::NoCandidates => "no other seat is in the pot",
            NoOpponent::Cancelled => "the request's equity was cancelled (the decision was superseded) before an opponent was chosen",
            NoOpponent::Overran => "the equity phase overran: its cutoff came before an opponent was chosen",
            NoOpponent::Incompatible => "no other seat's street-root public range holds a combo compatible with hero's: no pairwise-compatible equity exists",
            NoOpponent::NotComputed => "no opponent had computable pairwise-compatible equity within the budget",
        }
    }
}

/// Spec 6: the seat of `others` whose public range has the highest range-vs-range equity against hero's public range
/// (`equity::range_vs_range`, the opponent's range first: its equity), within `budget` from now (`select_opponent` on the
/// system clock). `None` when no seat was chosen (`select_opponent`'s causes).
pub fn choose_opponent(hero: Seat, hero_public: &Range1326, others: &[(Seat, Range1326)], board: &[Card], budget: Duration, cancel: &AtomicBool) -> Option<Seat> {
    let clock = SystemClock::new();
    let budget_ms = u64::try_from(budget.as_millis()).unwrap_or(u64::MAX);
    let cutoff_ms = clock.now_ms().saturating_add(budget_ms);
    select_opponent(&clock, cutoff_ms, hero, hero_public, others, board, cancel, &crate::equity::range_vs_range).ok()
}

/// Spec 6's opponent choice with one absolute cutoff `cutoff_ms` on `clock` (fix round 1, P4T11-I2; spec 7: every phase
/// receives only the remaining time): the seat of `others` (hero never) with the highest `estimate` of its public
/// range's equity against hero's; a tie keeps the seat that comes first. Seats with no holding compatible with one of
/// hero's (`ranges::jointly_compatible`) are not estimated. The phase left when selection starts is shared evenly among
/// the other seats; the clock is read again before each estimate, which gets at most the lesser of its share and what
/// is left, and none is started once the cutoff has come (or the token is set). A seat whose estimate has no value is
/// skipped, never guessed. A selection that finishes after the cutoff is refused (spec 6: the surrogate is skipped when
/// the equity phase overran), even with an opponent found.
#[allow(clippy::too_many_arguments)]
pub fn select_opponent(clock: &dyn Clock, cutoff_ms: u64, hero: Seat, hero_public: &Range1326, others: &[(Seat, Range1326)], board: &[Card], cancel: &AtomicBool,
    estimate: &dyn Fn(&Range1326, &Range1326, &[Card], Duration, &AtomicBool) -> Option<(f32, EquityMethod)>) -> Result<Seat, NoOpponent> {
    let candidates: Vec<&(Seat, Range1326)> = others.iter().filter(|(seat, _)| *seat != hero).collect();
    if candidates.is_empty() {
        return Err(NoOpponent::NoCandidates);
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(NoOpponent::Cancelled);
    }
    let compatible: Vec<&(Seat, Range1326)> = candidates.into_iter().filter(|(_, range)| jointly_compatible(hero_public, range)).collect();
    if compatible.is_empty() {
        return Err(NoOpponent::Incompatible);
    }
    let start_ms = clock.now_ms();
    if start_ms >= cutoff_ms {
        return Err(NoOpponent::Overran);
    }
    let seats = u64::try_from(compatible.len()).expect("at most five other seats");
    let share_ms = ((cutoff_ms - start_ms) / seats).max(1);
    let mut best: Option<(Seat, f32)> = None;
    for (seat, range) in compatible {
        if cancel.load(Ordering::SeqCst) {
            return Err(NoOpponent::Cancelled);
        }
        let now_ms = clock.now_ms();
        if now_ms >= cutoff_ms {
            return Err(NoOpponent::Overran);
        }
        let allowance = Duration::from_millis(share_ms.min(cutoff_ms - now_ms));
        let Some((equity, _method)) = estimate(range, hero_public, board, allowance, cancel) else { continue };
        match best {
            Some((_, leader)) if equity <= leader => {}
            _ => best = Some((*seat, equity)),
        }
    }
    if clock.now_ms() > cutoff_ms {
        return Err(NoOpponent::Overran);
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(NoOpponent::Cancelled);
    }
    best.map(|(seat, _)| seat).ok_or(NoOpponent::NotComputed)
}

/// Spec 6's synthetic root against `opponent` at `state`'s decision on `street` with `template_id`; `None` skips the
/// surrogate (an opponent all-in, a zero stack or pot, a seat not in the pot, another street than the decision's).
pub fn surrogate_input(d: &Derived, state: &HandState, hero: Seat, opponent: Seat, street: Street, template_id: &str) -> Option<SurrogateInput> {
    synthetic_root(d, state, hero, opponent, street, template_id).ok()
}

/// `surrogate_input` with the reason it is skipped, for the multiway `Final`'s note.
pub(crate) fn synthetic_root(d: &Derived, state: &HandState, hero: Seat, opponent: Seat, street: Street, template_id: &str) -> Result<SurrogateInput, String> {
    if street == Street::Preflop || d.street != street {
        return Err(format!("the synthetic root is on the decision's street ({:?}), not {street:?}", d.street));
    }
    if opponent == hero {
        return Err("hero is not its own opponent".into());
    }
    let seat = |s: Seat| usize::from(s.0);
    for s in [hero, opponent] {
        if !state.dealt.contains(&s) || d.folded.get(seat(s)).copied().unwrap_or(true) {
            return Err(format!("seat {} is not in the pot", s.0));
        }
    }
    if d.all_in[seat(opponent)] {
        return Err(format!("the chosen opponent, seat {}, is all-in", opponent.0));
    }
    let stack = d.stacks_remaining[seat(hero)].min(d.stacks_remaining[seat(opponent)]);
    if stack == 0 {
        return Err("the synthetic stack (the smaller remaining stack) is 0 chips".into());
    }
    if d.pot == 0 {
        return Err("the pot is 0 chips".into());
    }
    let order = core_model::postflop_order(state.button, &state.dealt);
    let at = |s: Seat| order.iter().position(|x| *x == s);
    let (Some(hero_at), Some(opponent_at)) = (at(hero), at(opponent)) else {
        return Err("hero or the opponent has no place in the postflop order".into());
    };
    Ok(SurrogateInput {
        hero,
        opponent,
        pot: d.pot,
        stack,
        hero_role: if hero_at < opponent_at { "oop" } else { "ip" },
        template_id: template_id.into(),
        pot_eligible: crate::coverage::pot_eligible(d),
    })
}

/// The live request a surrogate runs inside (spec 6, 7): its decision, its absolute deadlines, and its street deadline
/// and once-only `Final` claim, both shared with its watchdog since admission. The surrogate only reads them: it never
/// arms, disarms or re-arms the watchdog, and its terminal is never published to the street deadline.
pub struct SurrogateRequest {
    pub identity: DecisionIdentity,
    pub deadlines: Deadlines,
    pub street_deadline: Arc<StreetDeadline>,
    pub final_claim: Option<Arc<AtomicBool>>,
}

/// Why a surrogate has no block (the multiway `Final`'s note), and whether its solve left a job the worker may still be
/// running at the watchdog's fire (ruling 28-I3: the worker is killed once the `Final` is out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub why: String,
    pub outstanding_job: bool,
}

/// Spec 6's isolated solve of `input` over `ranges` (the street-root public ranges, OOP then IP) on `board`, and hero's
/// advice for `hero_cards` read from it (see the module doc). `bb_chips` and `rake` are the hand's, `target_bp` the
/// request's (captured at admission); the solve has what is left until `request`'s street deadline. The worker is
/// readied as for any solve (a pending cancel settled, a missing worker relaunched once), and the request goes through
/// the solve client's shared transport. `Skipped` names why no block is produced: a decision no longer active, a root
/// that does not materialize, a worker not ready, no time for one iteration, a failed, refused, hung, expired or
/// superseded solve, or hero's node or combo row missing from the solution.
#[allow(clippy::too_many_arguments)]
pub fn run_surrogate(core: &mut EngineCore, input: &SurrogateInput, ranges: [Range1326; 2], board: &[Card], hero_cards: [Card; 2], bb_chips: u32, rake: &Rake,
    target_bp: u16, request: &SurrogateRequest, sink: &SharedSink) -> Result<ExperimentalHu, Skipped> {
    let skip = |why: String| Skipped { why, outstanding_job: false };
    if bb_chips == 0 {
        return Err(skip("the big blind is 0 chips".into()));
    }
    let street = match board.len() {
        3 => Street::Flop,
        4 => Street::Turn,
        5 => Street::River,
        n => return Err(skip(format!("a {n}-card board is on no postflop street"))),
    };
    // A decision superseded before anything is built starts no work (ruling 22-I2).
    if !core.identity_active(&request.identity) {
        return Err(skip("superseded by a newer request".into()));
    }
    let hero_oop = input.hero_role == "oop";
    let (oop, ip) = if hero_oop { (input.hero, input.opponent) } else { (input.opponent, input.hero) };
    let root = StreetRootSnapshot { street, board: board.to_vec(), oop, ip, pot_root: input.pot, stack_oop_root: input.stack, stack_ip_root: input.stack,
        dead_this_street: 0, projected_from: input.pot_eligible, history: vec![], bb_chips };
    let build = build_tree_full(&root, &TemplateSelection::from_history(&input.template_id, &[]))
        .map_err(|reason| skip(format!("the synthetic root does not materialize ({})", miss_cause(&reason))))?;
    // The request's own deadlines, street deadline and claim; no `_min` retry; the wire's `background: false`.
    let plan = SolvePlan { identity: request.identity.clone(), deadlines: request.deadlines, street_deadline: request.street_deadline.clone(),
        template_id: input.template_id.clone(), retry_template_id: None, rake: *rake, hero_actor: input.hero_role.into(), background: false,
        final_claim: request.final_claim.clone() };
    let mut relaunches = 0u8;
    worker_for_request(core, &plan, &mut relaunches).map_err(|reason| skip(format!("the worker is not ready ({})", miss_cause(&reason))))?;
    let deadline_ms = plan.deadlines.worker_deadline_ms(core.clock.now_ms(), plan.deadlines.street_deadline_ms)
        .ok_or_else(|| skip("no time is left for one iteration before the street deadline".into()))?;
    core.set_stage("building");
    let req = solve_request_from_parts(core, &root, &ranges, target_bp, &plan, &build, deadline_ms);
    let out = send_solve_request(core, req, &plan, &build, sink);
    let solution = match (&out.terminal, out.solution) {
        (Terminal::Ok | Terminal::BestSoFar, Some(solution)) => solution,
        (Terminal::Failed(reason), _) => {
            return Err(Skipped { why: format!("the synthetic-root solve failed ({})", miss_cause(reason)), outstanding_job: out.outstanding_job });
        }
        (terminal, None) => unreachable!("send_solve_request: a {terminal:?} outcome without its solution"),
    };
    // Spec 6: hero's advice at the synthetic root when OOP, at the node after OOP's check when IP.
    let path: &[Action] = if hero_oop { &[] } else { &[Action::Check] };
    let ordinal = resolve_chip_path(&build.tree.materialized, path).ok_or_else(|| skip("the synthetic tree has no node after OOP's check".into()))?;
    let index = out.ordinal_paths.iter().position(|p| *p == ordinal).ok_or_else(|| skip("the solve did not export hero's node".into()))?;
    let node = &solution.nodes[index];
    if node.actor != input.hero_role {
        return Err(skip(format!("hero's node is {}'s, not {}'s", node.actor, input.hero_role)));
    }
    // Hero's actual combo row (never an empty reach vector); the solution was validated whole, a fold's EV exactly 0.
    let combo = usize::from(combo_index(hero_cards[0], hero_cards[1]));
    if !node.available[combo] {
        return Err(skip("hero's combo is out of support at hero's node of the synthetic tree".into()));
    }
    let bb = f64::from(bb_chips);
    let actions = node.actions.iter().enumerate()
        .map(|(a, action)| ActionAdvice { action: *action, frequency: Some(node.probs[combo][a]), ev_bb: Some((f64::from(node.ev_chips[combo][a]) / bb) as f32),
            unavailable: None, headline: false })
        .collect();
    let used = |seat: Seat, range: &Range1326| (seat, core_ranges::range_to_string(range), core_ranges::mass(range));
    Ok(ExperimentalHu { opponent: input.opponent, hero_role: input.hero_role.into(), pot: input.pot, stack: input.stack, template_id: input.template_id.clone(),
        ranges_used: [used(oop, &ranges[0]), used(ip, &ranges[1])], actions, reached_bp: out.reached_bp, elapsed_ms: out.elapsed_ms, note: EXPERIMENTAL_NOTE.into() })
}
