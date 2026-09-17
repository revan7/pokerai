//! Weighted range-vs-range equity (spec sections 3.5 and 7).
//!
//! Joint weighting: every tuple of pairwise-disjoint combos, one per player, carries the product
//! of its combos' weights; runouts are uniform over the remaining deck; a tie splits a pot equally
//! among the tied eligible players. Sums accumulate in `f64` and narrow to `f32` only at the
//! output boundary, so the result is deterministic and independent of player order.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use proto::{combo_cards, Card, ComboIndex, EquityMethod, Rake, Range1326, Seat, COMBOS};

use crate::evaluator::{BinaryEvaluator, Evaluator};

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerRange { pub seat: Seat, pub range: Range1326 }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EquityMode { Exact, MonteCarlo { seed: u64, max_samples: u32 } }

#[derive(Clone, Debug, PartialEq)]
pub struct PotEligibility { pub pot_index: u8, pub eligible: Vec<Seat> }

#[derive(Clone, Debug, PartialEq)]
pub struct EquityRequest { pub board: Vec<Card>, pub players: Vec<PlayerRange>, pub mode: EquityMode, pub pots: Vec<PotEligibility> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquityStatus { Ready, Cancelled, BudgetExceeded, InvalidRanges }

#[derive(Clone, Debug, PartialEq)]
pub struct EquityShare { pub pot_index: u8, pub seat: Seat, pub value: f32, pub std_err: f32 }

#[derive(Clone, Debug, PartialEq)]
pub struct EquityResult { pub status: EquityStatus, pub method: Option<EquityMethod>, pub shares: Vec<EquityShare>, pub samples: u64, pub elapsed: Duration }

impl EquityRequest {
    pub fn single_pot(board: Vec<Card>, players: Vec<PlayerRange>, mode: EquityMode) -> EquityRequest { EquityRequest { board, players, mode, pots: vec![] } }
    pub(crate) fn pot_list(&self) -> Vec<PotEligibility> {
        if self.pots.is_empty() { vec![PotEligibility { pot_index: 0, eligible: self.players.iter().map(|p| p.seat).collect() }] } else { self.pots.clone() }
    }
    pub(crate) fn board_used(&self) -> [bool; 52] { let mut used = [false; 52]; for c in &self.board { used[c.0 as usize] = true; } used }
}

/// Always-on preconditions of a board: at most five cards, every id in range, no repeats.
///
/// `EquityRequest::board_used` indexes a 52-slot array by card id and the runout enumeration
/// subtracts the board length from 5, so an invalid board is a panic waiting to happen several
/// frames deeper. `per_combo_equity` checks the board here because it can return before building
/// any `EquityRequest` (when hero has no supported combo) and would otherwise report all zeros
/// for an unusable board.
pub(crate) fn check_board(board: &[Card]) {
    assert!(board.len() <= 5, "equity: board of {} cards exceeds the 5-card maximum", board.len());
    let mut on_board = [false; 52];
    for (i, c) in board.iter().enumerate() {
        assert!(c.0 < 52, "equity: board card at index {i} has out-of-range id {}", c.0);
        assert!(!on_board[c.0 as usize], "equity: duplicate board card {c} at index {i}");
        on_board[c.0 as usize] = true;
    }
}

/// Always-on structural preconditions of a request.
///
/// These are caller bugs, not data conditions: a well-formed request whose ranges simply cannot
/// produce a showdown is reported as `EquityStatus::InvalidRanges` instead. Every check here
/// guards an arithmetic or indexing assumption the enumeration relies on: the board through
/// `check_board`, a duplicated seat that would emit two shares for one player, an ineligible or
/// repeated seat in a pot that `Tally` would mis-index, and a player count large enough to
/// underflow `52 - board - 2 * players` in `exact_cost` and in the runout enumeration. The cost
/// is bounded by the deck, not by the enumeration, so it is re-checked at every public entry
/// point rather than threaded through as a proof obligation. Weight-domain validation lives in
/// `support`, which already scans every range exactly once on both the exact and the Monte Carlo
/// path.
pub(crate) fn check_request(req: &EquityRequest) {
    check_board(&req.board);
    assert!(!req.players.is_empty(), "equity: a request needs at least one player");
    // Two hole cards per player plus the five-card final board must fit in one deck.
    assert!(
        2 * req.players.len() + 5 <= 52,
        "equity: {} players need more cards than the deck holds",
        req.players.len()
    );
    for (i, p) in req.players.iter().enumerate() {
        assert!(
            !req.players[..i].iter().any(|q| q.seat == p.seat),
            "equity: seat {} appears twice in players",
            p.seat.0
        );
    }
    for (k, pot) in req.pots.iter().enumerate() {
        assert!(!pot.eligible.is_empty(), "equity: pot {} has no eligible player", pot.pot_index);
        assert!(
            !req.pots[..k].iter().any(|q| q.pot_index == pot.pot_index),
            "equity: pot_index {} appears twice",
            pot.pot_index
        );
        for (i, s) in pot.eligible.iter().enumerate() {
            assert!(
                req.players.iter().any(|p| p.seat == *s),
                "equity: pot {} lists seat {} which is not a player",
                pot.pot_index,
                s.0
            );
            assert!(
                !pot.eligible[..i].iter().any(|t| t == s),
                "equity: pot {} lists seat {} twice",
                pot.pot_index,
                s.0
            );
        }
    }
}

pub(crate) type Support = Vec<(ComboIndex, [Card; 2], f64)>;

/// Supported combos of a range (weight > 0, not blocked by the board).
///
/// # Panics
/// Panics (in every build profile) if any weight is not finite in `[0, 1]`. `Range1326::from_fn`
/// and `Range1326::set` perform no validation, so a NaN can reach here from safe code; `w <= 0.0`
/// is false for NaN, so an unchecked NaN would flow into every `f64` accumulator and turn each
/// share into NaN. Callers ingesting untrusted weights validate before building the range
/// (`proto`'s `Range1326` deserializer and `core_ranges::parse_range` both already do).
pub(crate) fn support(range: &Range1326, board: &[Card]) -> Support {
    (0..COMBOS as u16).filter_map(|i| {
        let w = range.get(i);
        assert!(w.is_finite() && (0.0..=1.0).contains(&w), "support: combo {i} weight {w} is not finite in [0, 1]");
        if w <= 0.0 { return None; }
        let cards = combo_cards(i);
        if board.contains(&cards[0]) || board.contains(&cards[1]) { return None; }
        Some((i, cards, w as f64))
    }).collect()
}

/// `C(n, k)`, exact for the `n <= 52`, `k <= 5` range `check_request` admits (the running value
/// never exceeds `C(52, 4) * 48`, far below `u64::MAX`).
fn choose(n: u64, k: u64) -> u64 { if k > n { return 0; } (0..k).fold(1u64, |r, i| r * (n - i) / (i + 1)) }

/// Upper bound on the evaluations of an exact enumeration: product of support sizes times runouts.
///
/// Runouts are counted as unordered combinations of the remaining deck. The multiplications
/// saturate: a request large enough to overflow `u64` must never wrap into a small number that
/// the engine would read as cheap (spec section 7 selects exact enumeration when the cost is
/// `<= 2 * 10^7`).
///
/// # Panics
/// Panics on a structurally invalid request; see `check_request`.
pub fn exact_cost(req: &EquityRequest) -> u64 {
    check_request(req);
    let tuples: u64 = req
        .players
        .iter()
        .map(|p| support(&p.range, &req.board).len() as u64)
        .fold(1u64, u64::saturating_mul);
    let remaining = (52 - req.board.len() - 2 * req.players.len()) as u64;
    tuples.saturating_mul(choose(remaining, (5 - req.board.len()) as u64))
}

/// `equity * (pot - rake)`; rake = min(rate * pot, cap) for pot rake and 0 for a time charge
/// (spec section 6). `no_flop_no_drop` selects whether the caller rakes this pot at all, which
/// is a property of the hand's history, not of the amount: it never changes this formula.
pub fn terminal_payoff(equity: f32, pot: u32, rake: &Rake) -> f32 {
    let pot = pot as f32;
    let r = match rake { Rake::PotRake { rate, cap_mchips, .. } => (rate * pot).min(*cap_mchips as f32 / 1000.0), Rake::TimeCharge => 0.0 };
    equity * (pot - r)
}

/// Budget and cancellation clock.
///
/// Two counters, polled on the same 4096 cadence but kept apart on purpose:
/// `evals` counts rank evaluations and is what `EquityResult::samples` reports, while `work`
/// counts units of search that produce no evaluation at all. A search whose deeper players always
/// collide finishes no runout, so it would never reach an evaluation-only poll and could run for
/// an unbounded time past its budget (spec section 7, and the 50 ms overrun limit of section 13.1).
pub(crate) struct Deadline<'a> { start: Instant, budget: Duration, cancel: &'a AtomicBool, pub evals: u64, work: u64 }

impl<'a> Deadline<'a> {
    pub fn new(budget: Duration, cancel: &'a AtomicBool) -> Self { Deadline { start: Instant::now(), budget, cancel, evals: 0, work: 0 } }
    /// Unconditional poll: the cancel flag first, then the clock.
    ///
    /// The clock comparison is `>=`, not `>`, so a zero budget is exhausted whatever the timer's
    /// resolution reports on the first call. Used on entry to a run and again before any run
    /// reports a terminal status, so a cancelled or out-of-budget request never returns an answer
    /// the caller has already stopped waiting for.
    pub fn check(&self) -> Option<EquityStatus> {
        if self.cancel.load(Ordering::Relaxed) { return Some(EquityStatus::Cancelled); }
        if self.start.elapsed() >= self.budget { return Some(EquityStatus::BudgetExceeded); }
        None
    }
    /// Counts one evaluation; every 4096 evaluations checks the cancel flag and the clock.
    pub fn tick(&mut self) -> Option<EquityStatus> {
        self.evals += 1;
        if self.evals % 4096 == 0 { return self.check(); }
        None
    }
    /// Counts one unit of search work — one candidate combo visited, accepted or rejected — on the
    /// same cadence. Never touches `evals`, so `samples` stays a pure rank-evaluation count.
    pub fn tick_work(&mut self) -> Option<EquityStatus> {
        self.work += 1;
        if self.work % 4096 == 0 { return self.check(); }
        None
    }
    pub fn elapsed(&self) -> Duration { self.start.elapsed() }
}

/// Per-pot win accumulation: `acc[pot][player]`, `total[pot]`.
pub(crate) struct Tally { acc: Vec<Vec<f64>>, total: Vec<f64>, eligible_idx: Vec<Vec<usize>> }

impl Tally {
    pub fn new(req: &EquityRequest, pots: &[PotEligibility]) -> Tally {
        let n = req.players.len();
        let eligible_idx = pots.iter().map(|p| p.eligible.iter().map(|s| req.players.iter().position(|q| q.seat == *s).expect("every eligible seat is a player")).collect()).collect();
        Tally { acc: vec![vec![0.0; n]; pots.len()], total: vec![0.0; pots.len()], eligible_idx }
    }
    /// Awards one showdown of weight `w`: per pot the best eligible rank wins; ties split equally.
    ///
    /// Three allocation-free passes over the pot's eligible indices (best rank, count of holders,
    /// credit) rather than collecting the winners. This is the innermost statement of the whole
    /// enumeration and spec section 7 admits requests of up to `2 * 10^7` evaluations inside a
    /// 0.5 s budget, so one heap allocation per pot per showdown is not affordable here.
    pub fn award(&mut self, ranks: &[u16], w: f64) {
        for (k, idx) in self.eligible_idx.iter().enumerate() {
            let best = idx.iter().map(|i| ranks[*i]).max().expect("a pot has an eligible player");
            let winners = idx.iter().filter(|i| ranks[**i] == best).count();
            let each = w / winners as f64;
            for i in idx.iter().filter(|i| ranks[**i] == best) { self.acc[k][*i] += each; }
            self.total[k] += w;
        }
    }
    pub fn empty(&self) -> bool { self.total.iter().all(|t| *t == 0.0) }
    pub fn shares(&self, req: &EquityRequest, pots: &[PotEligibility], samples: Option<f64>) -> Vec<EquityShare> {
        let mut out = Vec::new();
        for (k, pot) in pots.iter().enumerate() {
            for i in &self.eligible_idx[k] {
                // Narrow to f32 only here, at the output boundary; the standard error is derived
                // from the wide ratio for the same reason.
                let ratio = self.acc[k][*i] / self.total[k];
                let std_err = samples.map(|n| ((ratio * (1.0 - ratio)) / n).sqrt() as f32).unwrap_or(0.0);
                out.push(EquityShare { pot_index: pot.pot_index, seat: req.players[*i].seat, value: ratio as f32, std_err });
            }
        }
        out
    }
}

pub(crate) fn result(status: EquityStatus, method: Option<EquityMethod>, shares: Vec<EquityShare>, samples: u64, elapsed: Duration) -> EquityResult {
    EquityResult { status, method, shares, samples, elapsed }
}

struct ExactRun<'a> {
    req: &'a EquityRequest,
    supports: Vec<Support>,
    used: [bool; 52],
    chosen: Vec<[Card; 2]>,
    ranks: Vec<u16>,
    tally: Tally,
    deadline: Deadline<'a>,
}

impl ExactRun<'_> {
    /// Depth-first over the players in request order; `used` keeps every tuple pairwise disjoint
    /// and disjoint from the board, so no invalid card set ever reaches the evaluator.
    fn assign(&mut self, player: usize, weight: f64) -> Option<EquityStatus> {
        if player == self.req.players.len() { return self.runouts(weight); }
        for idx in 0..self.supports[player].len() {
            // Every candidate counts, accepted or rejected. A search whose later seats always
            // collide completes no runout and evaluates no hand, so this is the only poll that
            // can observe the budget or the cancel flag on that path.
            if let Some(s) = self.deadline.tick_work() { return Some(s); }
            let (_, cards, w) = self.supports[player][idx];
            let (a, b) = (cards[0].0 as usize, cards[1].0 as usize);
            if self.used[a] || self.used[b] { continue; }
            self.used[a] = true; self.used[b] = true;
            self.chosen.push(cards);
            let stop = self.assign(player + 1, weight * w);
            self.chosen.pop();
            self.used[a] = false; self.used[b] = false;
            if stop.is_some() { return stop; }
        }
        None
    }

    /// Enumerates every runout for the chosen holes and awards each one with `weight`.
    fn runouts(&mut self, weight: f64) -> Option<EquityStatus> {
        let deck: Vec<Card> = (0..52u8).map(Card).filter(|c| !self.used[c.0 as usize]).collect();
        let k = 5 - self.req.board.len();
        let mut idx: Vec<usize> = (0..k).collect();
        let mut board_cards = self.req.board.clone();
        loop {
            board_cards.truncate(self.req.board.len());
            board_cards.extend(idx.iter().map(|i| deck[*i]));
            let full = BinaryEvaluator.partial(&board_cards);
            for (i, hole) in self.chosen.iter().enumerate() {
                self.ranks[i] = BinaryEvaluator.rank_with(&full, hole);
                if let Some(s) = self.deadline.tick() { return Some(s); }
            }
            self.tally.award(&self.ranks, weight);
            if k == 0 { return None; }
            let mut j = k;
            loop {
                if j == 0 { return None; }
                j -= 1;
                if idx[j] < deck.len() - k + j {
                    idx[j] += 1;
                    for t in (j + 1)..k { idx[t] = idx[t - 1] + 1; }
                    break;
                }
            }
        }
    }
}

pub(crate) fn exact(req: &EquityRequest, budget: Duration, cancel: &AtomicBool) -> EquityResult {
    check_request(req);
    let pots = req.pot_list();
    let deadline = Deadline::new(budget, cancel);
    // Entry poll, before any work: an already-cancelled or zero-budget request must never report
    // a result, and must never report `Ready`.
    if let Some(status) = deadline.check() { return result(status, None, vec![], 0, deadline.elapsed()); }
    let supports: Vec<Support> = req.players.iter().map(|p| support(&p.range, &req.board)).collect();
    if supports.iter().any(|s| s.is_empty()) {
        let status = deadline.check().unwrap_or(EquityStatus::InvalidRanges);
        return result(status, None, vec![], 0, deadline.elapsed());
    }
    let n = req.players.len();
    let mut run = ExactRun { req, supports, used: req.board_used(), chosen: Vec::with_capacity(n), ranks: vec![0; n], tally: Tally::new(req, &pots), deadline };
    let stop = run.assign(0, 1.0);
    let elapsed = run.deadline.elapsed();
    if let Some(status) = stop { return result(status, None, vec![], run.deadline.evals, elapsed); }
    // Final poll before reporting a terminal status. Without it a run that finished between two
    // polls could report `Ready` after cancellation, or `InvalidRanges` for a search that simply
    // ran out of budget before completing a single tuple.
    if let Some(status) = run.deadline.check() { return result(status, None, vec![], run.deadline.evals, elapsed); }
    if run.tally.empty() { return result(EquityStatus::InvalidRanges, None, vec![], run.deadline.evals, elapsed); }
    result(EquityStatus::Ready, Some(EquityMethod::Exact), run.tally.shares(req, &pots, None), run.deadline.evals, elapsed)
}

/// Spec 3.5 entry point: exact enumeration or time-bounded Monte Carlo per `req.mode`.
///
/// # Panics
/// Panics on a structurally invalid request; see `check_request`.
pub fn equity(req: &EquityRequest, budget: Duration, cancel: &AtomicBool) -> EquityResult {
    check_request(req);
    match req.mode {
        EquityMode::Exact => exact(req, budget, cancel),
        EquityMode::MonteCarlo { seed, max_samples } => crate::mc::monte_carlo(req, seed, max_samples, budget, cancel),
    }
}

/// Exact equity of every supported hero combo against the villain's disjoint weighted combos over
/// all runouts (river terminal use).
///
/// The villain range is read, never written: each hero combo is conditioned out of the villain's
/// combos by the enumeration's disjointness, so hero's cards never enter the public range itself
/// (spec section 2). Unsupported hero combos — zero weight, or blocked by the board — score 0.
///
/// # Panics
/// Panics on an invalid board (`check_board`) or an out-of-domain weight in either range
/// (`support`).
pub fn per_combo_equity(hero: &Range1326, villain: &Range1326, board: &[Card]) -> [f32; COMBOS] {
    check_board(board);
    let mut out = [0f32; COMBOS];
    let never = AtomicBool::new(false);
    for (i, _, _) in support(hero, board) {
        let one = Range1326::from_fn(|j| if j == i { 1.0 } else { 0.0 });
        let req = EquityRequest::single_pot(board.to_vec(), vec![PlayerRange { seat: Seat(0), range: one }, PlayerRange { seat: Seat(1), range: villain.clone() }], EquityMode::Exact);
        let res = exact(&req, Duration::from_secs(3600), &never);
        if res.status == EquityStatus::Ready { out[i as usize] = res.shares.iter().find(|s| s.seat == Seat(0)).map(|s| s.value).unwrap_or(0.0); }
    }
    out
}
