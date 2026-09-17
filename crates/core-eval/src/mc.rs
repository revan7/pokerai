//! Monte Carlo equity with joint disjoint sampling (spec sections 3.5, 13.1).
//!
//! The population is exactly the one the exact enumeration of `equity.rs` weights: each player's
//! combo is drawn proportionally to its weight, independently of the others, and the whole tuple is
//! rejected when any two combos share a card, so an accepted tuple carries the product of its
//! combos' weights over the disjoint tuples. The runout is a uniform partial shuffle of what is
//! left of the deck. Sampling is seeded, so the same request and seed reproduce the same answer.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use proto::{Card, EquityMethod};

use crate::equity::{check_request, result, support, Deadline, EquityRequest, EquityResult, EquityStatus, Support, Tally};
use crate::evaluator::{BinaryEvaluator, Evaluator};

/// A colliding request with no disjoint assignment must not sample forever; after this many
/// rejections without a single accepted tuple the request is `InvalidRanges`.
const REJECTION_CAP: u64 = 10_000_000;

/// Step cap of the compatibility search. A search that exceeds it is treated as compatible and
/// falls through to `REJECTION_CAP`, so the bounded search never turns a solvable request into
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

/// Bounded DFS: does any disjoint assignment exist? A search that exceeds the step cap counts as
/// compatible, so sampling (and its own rejection cap) decides instead of this approximation.
fn compatible(supports: &[Support], used: &mut [bool; 52], player: usize, steps: &mut u64) -> bool {
    if player == supports.len() { return true; }
    for (_, cards, _) in &supports[player] {
        *steps += 1;
        if *steps > DFS_STEP_CAP { return true; }
        let (a, b) = (cards[0].0 as usize, cards[1].0 as usize);
        if used[a] || used[b] { continue; }
        used[a] = true; used[b] = true;
        let ok = compatible(supports, used, player + 1, steps);
        used[a] = false; used[b] = false;
        if ok { return true; }
    }
    false
}

/// Test support: `count` accepted joint draws for the request's players. Bounded by the same
/// 10,000,000-rejection cap as `monte_carlo`, so a request with no disjoint assignment returns the
/// partial vector instead of spinning forever in the test binary.
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
                if rejections > REJECTION_CAP { break; }
            }
        }
    }
    out
}

/// Time-bounded Monte Carlo estimate of the same population the exact enumeration weights.
///
/// Status (spec section 7): `Ready` when `max_samples` is reached or the budget stops the loop with
/// at least one completed sample, `Cancelled` when the flag stops it (with shares when any sample
/// completed), `BudgetExceeded` only when no sample completed at all, and `InvalidRanges` when the
/// supports admit no disjoint assignment.
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
    if supports.iter().any(|s| s.is_empty()) { return result(EquityStatus::InvalidRanges, None, vec![], 0, deadline.elapsed()); }
    let mut probe = board_used;
    let mut steps = 0u64;
    if !compatible(&supports, &mut probe, 0, &mut steps) { return result(EquityStatus::InvalidRanges, None, vec![], 0, deadline.elapsed()); }
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
    let mut rejections = 0u64;
    let mut stop: Option<EquityStatus> = None;
    while samples < max_samples as u64 {
        let Some(used) = sampler.draw_into(&mut rng, &board_used, &mut holes) else {
            rejections += 1;
            if samples == 0 && rejections > REJECTION_CAP {
                return result(EquityStatus::InvalidRanges, None, vec![], 0, deadline.elapsed());
            }
            // A request whose players always collide completes no sample and so evaluates no rank:
            // this is the only poll that can observe the budget or the cancel flag on that path.
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
