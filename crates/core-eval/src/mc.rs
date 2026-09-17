//! Monte Carlo equity with joint disjoint sampling (spec sections 3.5, 13.1).
//!
//! The population is exactly the one the exact enumeration of `equity.rs` weights: each player's
//! combo is drawn proportionally to its weight, independently of the others, and the whole tuple is
//! rejected when any two combos share a card, so an accepted tuple carries the product of its
//! combos' weights over the disjoint tuples. The runout is a uniform partial shuffle of what is
//! left of the deck. Sampling is seeded, so the same request and seed reproduce the same answer.
//!
//! Every phase of a run - the compatibility search as much as the sampling loop - is inside the
//! caller's budget and answers the cancel flag (spec section 7, review T25-R1), and `InvalidRanges`
//! is a statement about the ranges alone: an empty support, or a completed proof that no disjoint
//! tuple exists. How long sampling takes to find a rare compatible tuple is a property of the
//! sampler, never evidence about the input (review T25-R2).

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use proto::{Card, EquityMethod};

use crate::equity::{check_request, result, support, Deadline, EquityRequest, EquityResult, EquityStatus, Support, Tally};
use crate::evaluator::{BinaryEvaluator, Evaluator};

/// Draw cap of `sample_joint_holes` alone, so a test binary asking for draws that its request can
/// almost never produce returns a short vector instead of hanging. It carries no status meaning:
/// `monte_carlo` has a budget and a cancel flag instead, and never reads a rejection count.
const HELPER_DRAW_CAP: u64 = 10_000_000;

/// Step cap of the compatibility search. A search that exceeds it ends as `Compat::Unknown` — it
/// proved nothing — and sampling decides, so the bounded search never turns a solvable request into
/// `InvalidRanges` by giving up early.
const DFS_STEP_CAP: u64 = 5_000_000;

/// xoshiro256** seeded through splitmix64; deterministic and dependency-free.
pub struct Xoshiro256 { s: [u64; 4] }

impl Xoshiro256 {
    pub fn seed(seed: u64) -> Xoshiro256 {
        let mut z = seed;
        let mut s = [0u64; 4];
        for slot in s.iter_mut() {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            *slot = x ^ (x >> 31);
        }
        Xoshiro256 { s }
    }
    pub fn next_u64(&mut self) -> u64 {
        let out = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        out
    }
    pub fn next_f64(&mut self) -> f64 { (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64) }
    pub fn below(&mut self, n: usize) -> usize { ((self.next_u64() as u128 * n as u128) >> 64) as usize }
}

/// Per-player supports with their prefix sums, for weighted draws.
struct Sampler { supports: Vec<Support>, cumulative: Vec<Vec<f64>> }

impl Sampler {
    fn new(supports: Vec<Support>) -> Sampler {
        let cumulative = supports.iter().map(|s| { let mut acc = 0.0; s.iter().map(|(_, _, w)| { acc += w; acc }).collect() }).collect();
        Sampler { supports, cumulative }
    }

    /// One combo of `player`, drawn with probability proportional to its weight.
    fn draw_one(&self, player: usize, rng: &mut Xoshiro256) -> [Card; 2] {
        let cum = &self.cumulative[player];
        let u = rng.next_f64() * cum[cum.len() - 1];
        // `next_f64` is in [0, 1), so `u` is below the total and `partition_point` always finds a
        // slot; the `min` only guards the rounding of the last prefix sum.
        let idx = cum.partition_point(|c| *c <= u).min(cum.len() - 1);
        self.supports[player][idx].1
    }

    /// One joint draw into `holes`; `None` when two players' combos overlap (rejection), in which
    /// case the remaining players are not drawn at all. Returns the card mask of board plus holes.
    fn draw_into(&self, rng: &mut Xoshiro256, board_used: &[bool; 52], holes: &mut Vec<[Card; 2]>) -> Option<[bool; 52]> {
        let mut used = *board_used;
        holes.clear();
        for p in 0..self.supports.len() {
            let cards = self.draw_one(p, rng);
            let (a, b) = (cards[0].0 as usize, cards[1].0 as usize);
            if used[a] || used[b] { return None; }
            used[a] = true; used[b] = true;
            holes.push(cards);
        }
        Some(used)
    }
}

/// Outcome of the bounded compatibility search. Only `Proven` is a statement about the ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compat {
    /// A pairwise-disjoint assignment was exhibited: some tuple can be sampled.
    Witness,
    /// The search was exhaustive and found none: no disjoint tuple exists. `InvalidRanges`.
    Proven,
    /// The step cap or a stop request ended the search before it could decide either way.
    Unknown,
}

/// Bounded DFS for a disjoint assignment, polling the deadline on every candidate it visits.
///
/// The search can visit up to `DFS_STEP_CAP` candidates, which is milliseconds of work on a request
/// whose whole budget may be half a second, so it is not a phase a caller can be made to wait out:
/// `tick_work` is called for every candidate (the same counter the rejection loop uses, because
/// neither produces a rank evaluation) and a stop ends the search as `Unknown` with the status
/// recorded in `stop` (review T25-R1). An `Unknown` from any child ends the whole search, since both
/// causes — the shared step counter and the clock — are global rather than local to one branch.
fn compatible(
    supports: &[Support],
    used: &mut [bool; 52],
    player: usize,
    steps: &mut u64,
    deadline: &mut Deadline<'_>,
    stop: &mut Option<EquityStatus>,
) -> Compat {
    if player == supports.len() { return Compat::Witness; }
    for (_, cards, _) in &supports[player] {
        *steps += 1;
        if *steps > DFS_STEP_CAP { return Compat::Unknown; }
        if let Some(s) = deadline.tick_work() { *stop = Some(s); return Compat::Unknown; }
        let (a, b) = (cards[0].0 as usize, cards[1].0 as usize);
        if used[a] || used[b] { continue; }
        used[a] = true; used[b] = true;
        let sub = compatible(supports, used, player + 1, steps, deadline, stop);
        used[a] = false; used[b] = false;
        match sub {
            Compat::Witness => return Compat::Witness,
            Compat::Unknown => return Compat::Unknown,
            // This candidate leads nowhere, but the branch was searched exhaustively: try the next.
            Compat::Proven => {}
        }
    }
    Compat::Proven
}

/// Test support: `count` accepted joint draws for the request's players.
///
/// Bounded by `HELPER_DRAW_CAP` rejections, so a request whose compatible tuples are unreachable by
/// sampling returns a short vector instead of hanging a test binary. That bound belongs to this
/// helper alone and says nothing about the request: it is not `monte_carlo`'s stopping rule, and it
/// is never a reason to call a range invalid (review T25-R2). A caller that needs exactly `count`
/// draws asserts on the returned length.
///
/// # Panics
/// Panics on a structurally invalid request; see `check_request`.
pub fn sample_joint_holes(req: &EquityRequest, seed: u64, count: usize) -> Vec<Vec<[Card; 2]>> {
    check_request(req);
    let sampler = Sampler::new(req.players.iter().map(|p| support(&p.range, &req.board)).collect());
    if sampler.supports.iter().any(|s| s.is_empty()) { return vec![]; }
    let mut rng = Xoshiro256::seed(seed);
    let board_used = req.board_used();
    let mut out = Vec::with_capacity(count);
    let mut holes = Vec::with_capacity(req.players.len());
    let mut rejections = 0u64;
    while out.len() < count {
        match sampler.draw_into(&mut rng, &board_used, &mut holes) {
            Some(_) => out.push(holes.clone()),
            None => {
                rejections += 1;
                if rejections > HELPER_DRAW_CAP { break; }
            }
        }
    }
    out
}

/// Time-bounded Monte Carlo estimate of the same population the exact enumeration weights.
///
/// Status (spec section 7): `Ready` when `max_samples` is reached or the budget stops the loop with
/// at least one completed sample, `Cancelled` when the flag stops it (with shares when any sample
/// completed), `BudgetExceeded` only when no sample completed at all, and `InvalidRanges` only when
/// a support is empty or the compatibility search completed a proof that no disjoint tuple exists.
/// A request whose compatible tuples are too rare to sample is not invalid: it samples until the
/// budget or the flag stops it and reports that (review T25-R2).
///
/// # Panics
/// Panics on a structurally invalid request; see `check_request`.
pub fn monte_carlo(req: &EquityRequest, seed: u64, max_samples: u32, budget: Duration, cancel: &AtomicBool) -> EquityResult {
    check_request(req);
    let pots = req.pot_list();
    let mut deadline = Deadline::new(budget, cancel);
    // Entry poll, before any work: an already-cancelled or zero-budget request must never compute,
    // and must never report `Ready`.
    if let Some(status) = deadline.check() { return result(status, None, vec![], 0, deadline.elapsed()); }
    let supports: Vec<Support> = req.players.iter().map(|p| support(&p.range, &req.board)).collect();
    let board_used = req.board_used();
    // Both preflight terminals report a stop in preference to `InvalidRanges`, as the exact path
    // does: a caller that has stopped waiting is told so, and is never told its ranges are the
    // problem on the strength of a search that did not finish.
    if supports.iter().any(|s| s.is_empty()) {
        let status = deadline.check().unwrap_or(EquityStatus::InvalidRanges);
        return result(status, None, vec![], 0, deadline.elapsed());
    }
    let mut probe = board_used;
    let mut steps = 0u64;
    let mut probe_stop: Option<EquityStatus> = None;
    let compat = compatible(&supports, &mut probe, 0, &mut steps, &mut deadline, &mut probe_stop);
    if let Some(status) = probe_stop { return result(status, None, vec![], 0, deadline.elapsed()); }
    if compat == Compat::Proven {
        let status = deadline.check().unwrap_or(EquityStatus::InvalidRanges);
        return result(status, None, vec![], 0, deadline.elapsed());
    }
    let sampler = Sampler::new(supports);
    let mut rng = Xoshiro256::seed(seed);
    let mut tally = Tally::new(req, &pots);
    let n = req.players.len();
    let k = 5 - req.board.len();
    let deck_base: Vec<Card> = (0..52u8).map(Card).filter(|c| !board_used[c.0 as usize]).collect();
    let mut ranks = vec![0u16; n];
    let mut holes: Vec<[Card; 2]> = Vec::with_capacity(n);
    let mut deck: Vec<Card> = Vec::with_capacity(deck_base.len());
    let mut board_cards = req.board.clone();
    let mut samples = 0u64;
    let mut stop: Option<EquityStatus> = None;
    while samples < max_samples as u64 {
        let Some(used) = sampler.draw_into(&mut rng, &board_used, &mut holes) else {
            // A rejected draw completes no sample and so evaluates no rank: this is the only poll
            // that can observe the budget or the cancel flag on that path. How many rejections it
            // takes to find a rare compatible tuple is not counted and cannot end the run: only the
            // budget, the flag or `max_samples` does (review T25-R2).
            if let Some(s) = deadline.tick_work() { stop = Some(s); break; }
            continue;
        };
        // Partial Fisher-Yates over what neither the board nor any hole card uses: the first `k`
        // entries are a uniform unordered runout.
        deck.clear();
        deck.extend(deck_base.iter().copied().filter(|c| !used[c.0 as usize]));
        for j in 0..k { let r = j + rng.below(deck.len() - j); deck.swap(j, r); }
        board_cards.truncate(req.board.len());
        board_cards.extend_from_slice(&deck[..k]);
        let full = BinaryEvaluator.partial(&board_cards);
        for (i, hole) in holes.iter().enumerate() {
            ranks[i] = BinaryEvaluator.rank_with(&full, hole);
            // Record the stop but finish the tuple: a half-evaluated showdown must not be awarded.
            if let Some(s) = deadline.tick() { stop = Some(s); }
        }
        tally.award(&ranks, 1.0);
        samples += 1;
        if stop.is_some() { break; }
    }
    let elapsed = deadline.elapsed();
    if samples == 0 { return result(stop.unwrap_or(EquityStatus::BudgetExceeded), None, vec![], 0, elapsed); }
    let shares = tally.shares(req, &pots, Some(samples as f64));
    let std_err = shares.iter().map(|s| s.std_err).fold(0.0f32, f32::max);
    let status = match stop { Some(EquityStatus::Cancelled) => EquityStatus::Cancelled, _ => EquityStatus::Ready };
    result(status, Some(EquityMethod::MonteCarlo { samples: samples as u32, std_err }), shares, samples, elapsed)
}
