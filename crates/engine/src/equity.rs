//! The equity adapter: the only engine file that builds `core-eval`'s request type (spec sections
//! 3.5, 4.4 and 7).
//!
//! Two populations, both named in spec section 4.4 and never mixed:
//! * hero's **actual combo** against one opponent's hero-conditioned public range
//!   (`hero_combo_equity`), which is also what the section 6 terminal calculations use;
//! * hero's **public range** against one opponent's public range (`range_vs_range`).
//!
//! Hero's cards never enter a public range, a solve input or a cache key (spec section 2). The
//! first population needs them, so it reads a private hero-conditioned copy from
//! `core_ranges::hero_conditioned` and writes a one-combo range of its own; the opponent's range
//! itself is only ever read.
//!
//! Mode selection is not this crate's formula: the engine prices the request with
//! `core_eval::exact_cost` and enumerates exactly iff that price is at most [`EXACT_COST_LIMIT`]
//! (spec section 7), falling back to a fixed-seed Monte Carlo run so the same decision always
//! produces the same numbers. `budget` is the time still available for one estimate; the caller
//! owns the absolute deadline (`clock::Clock::now_ms`) and passes what remains of it, defaulting to
//! [`EQUITY_BUDGET_MS`].

use crate::clock::{Clock, SystemClock};
use core_eval::{equity, exact_cost, EquityMode, EquityRequest, EquityStatus, PlayerRange};
use core_ranges::hero_conditioned;
use proto::{combo_index, Availability, Card, EquityEstimate, EquityMethod, EquitySummary, Range1326, Seat};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// The equity phase's whole budget in milliseconds: spec section 7 gives the phase 0.5 s after
/// `Fast`, cancellable — not 0.5 s per estimate (review R1).
pub const EQUITY_BUDGET_MS: u64 = 500;
/// Spec section 7 / review S3: the engine chooses exact enumeration iff `exact_cost(&req) <= 2 * 10^7`.
pub const EXACT_COST_LIMIT: u64 = 20_000_000;
/// Fixed Monte Carlo seed: repeating a decision must repeat its numbers exactly (spec section 5).
const MC_SEED: u64 = 7;
const MC_MAX_SAMPLES: u32 = 200_000;
/// Seat labels used only inside this module: hero is 0, the single opponent is 1.
const HERO: Seat = Seat(0);
const VILLAIN: Seat = Seat(1);

/// Why a run produced no equity value.
///
/// Crate-internal, and deliberately richer than the `Option` the public helpers return: a budget
/// overrun, a cancellation and a range that cannot reach a showdown are three different facts, and
/// spec section 2 forbids reporting one as another. [`estimate`] turns each into its own
/// `Availability::Unavailable` reason and `allin::facing_allin` into its own `UnsupportedReason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoEquity {
    /// Hero's cards are not entered, so there is no hero combo to run.
    NoHeroCombo,
    /// The equity phase's one deadline passed before this estimate was started (review R1). It is
    /// not `BudgetExceeded`: no work was done and no partial answer exists.
    DeadlineExpired,
    /// Empty support, or a completed proof that the two ranges cannot meet at a showdown.
    InvalidRanges,
    /// The budget ran out before an answer (never confused with `InvalidRanges`).
    BudgetExceeded,
    /// The caller cancelled before an answer.
    Cancelled,
    /// A `Ready` result that carried no hero share or no method: a `core-eval` contract break.
    Malformed(&'static str),
}

impl NoEquity {
    /// The operator-facing reason, claiming no more than what happened.
    pub(crate) fn reason(self) -> String {
        match self {
            NoEquity::NoHeroCombo => "hero's cards are not entered".to_string(),
            NoEquity::DeadlineExpired => "equity deadline expired before this estimate".to_string(),
            NoEquity::InvalidRanges => "no compatible holdings".to_string(),
            NoEquity::BudgetExceeded => "equity budget exceeded".to_string(),
            NoEquity::Cancelled => "cancelled before an answer".to_string(),
            NoEquity::Malformed(what) => format!("malformed equity result: {what}"),
        }
    }
}

/// One heads-up run: price exact enumeration, then run the mode that price selects.
///
/// The request is built once and its mode switched in place. `exact_cost` only reads the request,
/// and building a second one would copy both 1,326-weight ranges again for nothing.
fn run(hero: Range1326, opp: Range1326, board: &[Card], budget: Duration, cancel: &AtomicBool) -> Result<(f32, EquityMethod), NoEquity> {
    let mut req = EquityRequest::single_pot(
        board.to_vec(),
        vec![PlayerRange { seat: HERO, range: hero }, PlayerRange { seat: VILLAIN, range: opp }],
        EquityMode::Exact,
    );
    if exact_cost(&req) > EXACT_COST_LIMIT {
        req.mode = EquityMode::MonteCarlo { seed: MC_SEED, max_samples: MC_MAX_SAMPLES };
    }
    let res = equity(&req, budget, cancel);
    // Exhaustive on purpose: a new `EquityStatus` must break this build rather than silently
    // become "no compatible holdings".
    match res.status {
        EquityStatus::Ready => {}
        EquityStatus::InvalidRanges => return Err(NoEquity::InvalidRanges),
        EquityStatus::BudgetExceeded => return Err(NoEquity::BudgetExceeded),
        EquityStatus::Cancelled => return Err(NoEquity::Cancelled),
    }
    let value = res
        .shares
        .iter()
        .find(|s| s.pot_index == 0 && s.seat == HERO)
        .map(|s| s.value)
        .ok_or(NoEquity::Malformed("no hero share for pot 0"))?;
    let method = res.method.ok_or(NoEquity::Malformed("a ready result with no method"))?;
    Ok((value, method))
}

/// [`hero_combo_equity`] with the failure reason kept (crate-internal; see [`NoEquity`]).
pub(crate) fn hero_combo_run(hero: [Card; 2], opp_public: &Range1326, board: &[Card], budget: Duration, cancel: &AtomicBool) -> Result<(f32, EquityMethod), NoEquity> {
    // Hero's side of this population is one combo at weight 1; the opponent's copy is
    // hero-conditioned, so hero's cards are removed from it without touching the public range.
    let mut fixed = Range1326::zero();
    fixed.set(combo_index(hero[0], hero[1]), 1.0);
    run(fixed, hero_conditioned(opp_public, hero), board, budget, cancel)
}

/// Hero's actual combo against one opponent's public range, hero-conditioned (spec section 4.4).
pub fn hero_combo_equity(hero: [Card; 2], opp_public: &Range1326, board: &[Card], budget: Duration, cancel: &AtomicBool) -> Option<(f32, EquityMethod)> {
    hero_combo_run(hero, opp_public, board, budget, cancel).ok()
}

/// [`range_vs_range`] with the failure reason kept (crate-internal; see [`NoEquity`]).
fn range_run(hero_public: &Range1326, opp_public: &Range1326, board: &[Card], budget: Duration, cancel: &AtomicBool) -> Result<(f32, EquityMethod), NoEquity> {
    run(hero_public.clone(), opp_public.clone(), board, budget, cancel)
}

/// Hero's public range against one opponent's public range (spec section 4.4). Hero's actual cards
/// are not involved on either side, so neither range is conditioned on them.
pub fn range_vs_range(hero_public: &Range1326, opp_public: &Range1326, board: &[Card], budget: Duration, cancel: &AtomicBool) -> Option<(f32, EquityMethod)> {
    range_run(hero_public, opp_public, board, budget, cancel).ok()
}

fn estimate(r: Result<(f32, EquityMethod), NoEquity>) -> EquityEstimate {
    match r {
        Ok((value, method)) => EquityEstimate { value: Some(value), availability: Availability::Ready, method: Some(method) },
        Err(e) => EquityEstimate { value: None, availability: Availability::Unavailable { reason: e.reason() }, method: None },
    }
}

/// The summary to show while the estimates are still running (spec section 5's progressive fill).
pub fn pending_summary(opponents: &[Seat]) -> EquitySummary {
    let p = || EquityEstimate { value: None, availability: Availability::Pending, method: None };
    EquitySummary {
        hero_combo_vs_each: opponents.iter().map(|s| (*s, p())).collect(),
        hero_range_vs_each: opponents.iter().map(|s| (*s, p())).collect(),
        per_pot_shares: vec![],
    }
}

/// Spec section 4.4's two populations for every opponent: hero's actual combo against each seat's
/// hero-conditioned public range, and hero's public range against each public range.
///
/// `budget` is **one deadline for the whole summary**, not a per-estimate allowance: spec section 7
/// gives the equity phase 0.5 s and says "every phase receives only the remaining time" (review
/// R1). Each estimate is handed what is left of it, a finished estimate is kept, and an estimate
/// that never started because the deadline passed is reported `Unavailable` with that reason rather
/// than run on a renewed budget. `per_pot_shares` is filled by the multiway path, not here.
pub fn equity_summary(hero: Option<[Card; 2]>, hero_public: &Range1326, opponents: &[(Seat, Range1326)], board: &[Card], budget: Duration, cancel: &AtomicBool) -> EquitySummary {
    equity_summary_with_clock(&SystemClock::new(), hero, hero_public, opponents, board, budget, cancel)
}

/// [`equity_summary`] against a caller-supplied clock: the engine's single time source, so the
/// deadline is measurable in a test instead of raced against wall time (`clock.rs`, and nothing in
/// the engine calls `Instant::now` itself).
pub fn equity_summary_with_clock(clock: &dyn Clock, hero: Option<[Card; 2]>, hero_public: &Range1326, opponents: &[(Seat, Range1326)], board: &[Card], budget: Duration, cancel: &AtomicBool) -> EquitySummary {
    // Absolute deadline, taken once. `as_millis` is `u128`, so the budget is validated in its wide
    // form before narrowing (plan-1 standing ruling (a)) rather than clamped; the clock's own
    // resolution is milliseconds.
    let budget_ms = u64::try_from(budget.as_millis()).expect("equity: the summary budget in milliseconds must fit u64");
    let deadline_ms = clock.now_ms().saturating_add(budget_ms);
    let remaining = || {
        let now = clock.now_ms();
        if now >= deadline_ms { None } else { Some(Duration::from_millis(deadline_ms - now)) }
    };
    let combo = opponents
        .iter()
        .map(|(s, r)| {
            let run = match hero {
                None => Err(NoEquity::NoHeroCombo),
                Some(h) => match remaining() {
                    None => Err(NoEquity::DeadlineExpired),
                    Some(left) => hero_combo_run(h, r, board, left, cancel),
                },
            };
            (*s, estimate(run))
        })
        .collect();
    let range = opponents
        .iter()
        .map(|(s, r)| {
            let run = match remaining() {
                None => Err(NoEquity::DeadlineExpired),
                Some(left) => range_run(hero_public, r, board, left, cancel),
            };
            (*s, estimate(run))
        })
        .collect();
    EquitySummary { hero_combo_vs_each: combo, hero_range_vs_each: range, per_pot_shares: vec![] }
}
