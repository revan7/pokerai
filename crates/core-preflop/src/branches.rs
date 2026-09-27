//! Shared history branches and their Bayesian conditioning kernel (spec sections 8.4, 9.1, 9.2).
//!
//! Replay keeps **one ordered list of history branches** shared by every seat. Branch `k` carries
//! the observed history with every translated wager replaced by its mapped menu action
//! (`translated`), a branch weight `q_k` (`f64`, un-normalized, `1` for the initial branch, never
//! rescaled) and, per seat `S`, that seat's node position in `k` and its un-normalized per-combo
//! masses `w_{S,k}[c]` (every seat starts at `w[c] = 1`). These types live here, in
//! `core-preflop`, so that translated-node assembly can read them without a reverse dependency on
//! `core-replay`, which re-exports them unchanged (plan 3 global constraints).
//!
//! # Binding invariant and formulas (spec section 8.4)
//!
//! - Public marginal: `r_S[c] = sum_k q_k * w_{S,k}[c]` ([`marginal`]).
//! - Branch posterior: `pi_{S,k}[c] = q_k * w_{S,k}[c] / r_S[c]`, defined where `r_S[c] > 0`
//!   ([`posterior`]).
//! - Equal-total invariant: for every seat, `sum_c w_{S,k}[c]` is the same in every branch, so a
//!   branch's whole weight lives in `q_k`, and a seat that has not acted since `k` was created has
//!   `pi_{S,k}[c] = q_k / sum_j q_j` for every combo.
//! - On-menu action `a` by seat `V` in branch `k` ([`condition`] with `factor = 1`): with
//!   `M_k = sum_c w_{V,k}[c] * P_k(a|c) / sum_c w_{V,k}[c]` (the range-integrated likelihood,
//!   from the pre-update masses; it never sees hero's cards), `w_{V,k}[c] *= P_k(a|c) / M_k` and
//!   `q_k *= M_k`. Other seats' masses do not change. The division by `M_k` keeps the branch
//!   total unchanged, so the integrated likelihood enters exactly once, through `q_k`; omitting
//!   `/ M_k` conditions twice. `M_k = 0` removes the branch for every seat (`None`).
//! - Off-menu wager by `V` in branch `k`, split over the two adjacent menu sizes `X in {A, B}` of
//!   the likelihood interpolation (P3.T10's `interpolate` choices `f_X`): each child is
//!   [`condition`] with `factor = f_X`, i.e. `q_{kX} = q_k * f_X * M_{kX}` and
//!   `w_{V,kX}[c] = w_{V,k}[c] * P_k(X|c) / M_{kX}`; every other seat's masses are copied
//!   unchanged, and the caller advances the child's shared translated history (and every seat's
//!   next node) along `X`. Thus
//!   `sum_X q_{kX} * w_{V,kX}[c] = q_k * w_{V,k}[c] * (f_A * P_A[c] + f_B * P_B[c])`: `V`'s
//!   marginal is conditioned exactly once by the interpolated likelihood. A child with
//!   `M_{kX} = 0` is not created.
//! - Rescale ([`rescale`]): after every observed action, each seat's masses in **every** branch
//!   (residual and stopped included) are divided by one common factor `m_S = max_c r_S[c]`, and
//!   `ln(m_S)` -- the log of the removed maximum, not its negation -- is added to
//!   `log_reach[S]`. A factor common to a seat's branches changes no posterior and no `q`.
//! - Output boundary ([`range_output`]): the one place replay narrows `f64` to `f32`. A positive
//!   marginal below `f32::MIN_POSITIVE` becomes `f32::MIN_POSITIVE`; zero stays zero. There is no
//!   positive-reach threshold anywhere (spec section 9.2).
//!
//! # Transactional updates (for the replay walk that calls this kernel)
//!
//! One observed action is one transaction over the whole list: the caller builds every applying
//! branch's candidate children first, and only then replaces the list. If every **applying**
//! branch (live, with a node for the actor) has zero integrated support, the update is rejected:
//! the pre-action branches are kept unchanged and
//! `UnconditionedPriorStreet{cause: "zero support after <action>"}` is added (spec section 9.2).
//! Residual and stopped branches are frozen -- [`condition`] returns them unchanged -- so their
//! mass must never count as support that makes an otherwise impossible action look supported.
//!
//! # Numeric domain
//!
//! All arithmetic is `f64`. Out-of-domain inputs (a likelihood or factor outside `[0, 1]`, a
//! non-finite or negative mass, a wrong-length vector) are errors -- always-on assertions naming
//! the offending index -- never clamped into range, and every update checks its own mass
//! accounting (always on) within [`MASS_TOLERANCE`].

use proto::{Action, Range1326, Seat, COMBOS};

/// Relative tolerance of the always-on mass-accounting checks: a conditioned branch keeps the
/// actor's pre-update mass total, and a rescaled seat's marginal has maximum 1. Both are exact in
/// real arithmetic; in `f64` the drift over a 1326-term sum is a few `1e-13` at most, so a
/// larger drift is a broken invariant, not rounding. This is also the only allowance above `1.0`
/// that [`range_output`] accepts, and it is far below the `f32` half-ulp at `1.0` (`6e-8`), so
/// such a value narrows to exactly `1.0_f32`.
pub const MASS_TOLERANCE: f64 = 1e-10;

/// One seat's state in one history branch (spec section 9.1).
#[derive(Clone, Debug)]
pub struct SeatMass {
    pub seat: Seat,
    /// Store node key or snapshot ordinal path of this seat's next action in this branch; `None`
    /// in the residual and in a stopped branch.
    pub node: Option<String>,
    /// `w_{S,k}` of spec section 8.4: 1326 un-normalized weights, equal total across the seat's
    /// branches.
    pub mass: Vec<f64>,
}

/// One shared history branch (spec section 9.1).
#[derive(Clone, Debug)]
pub struct HistoryBranch {
    /// Creation order.
    pub id: u8,
    pub parent: Option<u8>,
    pub split_by: Option<Seat>,
    /// The history with each translated wager replaced by its mapped menu action; empty for the
    /// residual.
    pub translated: Vec<(Seat, Action)>,
    /// Shared branch weight of spec section 8.4: `1.0` for the initial branch, un-normalized,
    /// never rescaled.
    pub q: f64,
    /// The single frozen residual branch of spec section 8.4: no node for any seat, never
    /// conditioned.
    pub residual: bool,
    /// Cause when the branch stopped on the current street ("missing node <key>", spec section
    /// 9.3): `q` and every seat's masses frozen, no node for any seat.
    pub stopped: Option<String>,
    /// One entry per dealt seat.
    pub seats: Vec<SeatMass>,
}

/// The replay start state (spec section 9.2): one initial branch with `q = 1` in which every
/// dealt seat has the uniform masses (weight 1 on all 1326 combos). Hero's cards are never
/// applied.
pub fn initial(seats: &[Seat]) -> Vec<HistoryBranch> {
    vec![HistoryBranch {
        id: 0,
        parent: None,
        split_by: None,
        translated: vec![],
        q: 1.,
        residual: false,
        stopped: None,
        seats: seats.iter().map(|&seat| SeatMass { seat, node: None, mass: vec![1.; COMBOS] }).collect(),
    }]
}

/// Applies one observed action by `actor` to branch `b` exactly once (spec section 8.4) and
/// returns the conditioned child, or `None` when the child does not exist.
///
/// `p[c]` is `P_k(a | c)`, the likelihood of the action (on-menu) or of one mapped menu size `X`
/// (off-menu split) for combo `c` at the actor's node in this branch; `factor` is `1` for an
/// on-menu action and the interpolation frequency `f_X` for one child of an off-menu split. With
/// `M = sum_c w[c] * p[c] / sum_c w[c]` over the actor's pre-update masses, the child has
/// `q = b.q * factor * M` and actor masses `w[c] * p[c] / M`; every other seat's masses, the
/// translated history and every node position are copied unchanged (the caller advances them).
///
/// Returns `None` -- the child is not created, for every seat -- when `actor` is not a seat of
/// the branch, the actor has no mass in it, `M = 0`, or `factor = 0`. A residual or stopped
/// branch is frozen: it is returned unchanged (its `q` and masses are never conditioned), and
/// that `Some` is **not** evidence that the action has support (see the module docs).
///
/// # Panics
/// Always (not only in debug builds), naming the offending index, if `p` does not have 1326
/// entries, any `p[c]` or `factor` is not a finite value in `[0, 1]`, the branch weight is not a
/// finite value in `[0, 1]`, any actor mass is negative or non-finite, the result is not finite,
/// a positive branch weight or a positively supported mass underflows to zero (there is no
/// positive-reach threshold, so an underflow is an error rather than a silent zero), or the
/// actor's mass total is not conserved within [`MASS_TOLERANCE`].
pub fn condition(b: &HistoryBranch, actor: Seat, p: &[f64], factor: f64) -> Option<HistoryBranch> {
    assert!(p.len() == COMBOS, "condition: likelihood has {} entries, expected {COMBOS}", p.len());
    for (c, pc) in p.iter().enumerate() {
        assert!(pc.is_finite() && (0.0..=1.0).contains(pc), "condition: likelihood[{c}] = {pc} is not a probability in [0, 1]");
    }
    assert!(factor.is_finite() && (0.0..=1.0).contains(&factor), "condition: factor {factor} is not a probability in [0, 1]");
    validate_branch_weight(b, "condition");
    if b.residual || b.stopped.is_some() {
        return Some(b.clone());
    }
    let index = b.seats.iter().position(|s| s.seat == actor)?;
    let w = &b.seats[index].mass;
    check_masses(b, index, "condition");
    let total: f64 = w.iter().sum();
    if total == 0. {
        return None;
    }
    // R1 fix: accumulate the integrated likelihood with a shared power-of-two scale split across
    // both factors of every term, so a representable positive support is never lost to a per-term
    // underflow that a naive term-by-term multiply-then-sum would round to zero before the sum
    // ever sees it (see the module docs). Zero support is decided from the mass/likelihood
    // operands themselves -- never from whether the accumulated sum happens to compute to zero.
    let scale_half = pow2(350);
    let mut support_scaled = 0.0_f64;
    let mut any_positive = false;
    for (w, p) in w.iter().zip(p) {
        if *w > 0.0 && *p > 0.0 {
            any_positive = true;
        }
        support_scaled += (w * scale_half) * (p * scale_half);
    }
    let support = support_scaled / (scale_half * scale_half);
    if !any_positive || factor == 0. {
        return None;
    }
    assert!(
        support > 0.0,
        "condition: branch {} seat {actor:?} integrated support underflowed to 0 despite a positive mass and likelihood on some combo (the exact product is smaller than the smallest representable positive f64)",
        b.id
    );
    let m = support / total;
    assert!(
        m > 0.0 && m <= 1.0,
        "condition: branch {} seat {actor:?} integrated likelihood M = {m} from support {support} over total {total} is not in (0, 1]",
        b.id
    );
    let mut child = b.clone();
    child.q *= factor * m;
    assert!(
        child.q > 0.0 || b.q == 0.0,
        "condition: branch {} weight underflowed to 0 from q = {} * factor {factor} * M {m}",
        b.id,
        b.q
    );
    for (c, (w, p)) in child.seats[index].mass.iter_mut().zip(p).enumerate() {
        let before = *w;
        *w *= p / m;
        assert!(
            *w > 0.0 || before == 0.0 || *p == 0.0,
            "condition: branch {} seat {actor:?} mass[{c}] = {before} underflowed to 0 under likelihood {p} / M {m}",
            b.id
        );
    }
    check_masses(&child, index, "condition (conditioned child)");
    let after: f64 = child.seats[index].mass.iter().sum();
    assert!(
        (after - total).abs() <= MASS_TOLERANCE * total,
        "condition: branch {} seat {actor:?} mass total moved from {total} to {after} (M = {m})",
        b.id
    );
    Some(child)
}

/// An exact power of two (`2^e`), built directly from its IEEE-754 bit pattern so the scaled
/// accumulation in [`condition`] and [`marginal`] never introduces its own rounding error -- only
/// the caller's real quantities round, never the scale factor itself. Valid for `-1022 <= e <=
/// 1023` (the normal exponent range); this module only ever calls it with `e = 350`.
fn pow2(e: i32) -> f64 {
    f64::from_bits(((e + 1023) as u64) << 52)
}

/// Asserts that branch `b`'s shared weight `q` is a finite value in `[0, 1]` (spec section 8.4),
/// naming the branch id and the calling context. Shared by every reader of a branch weight --
/// [`condition`] (checked before its frozen early return, so a residual or stopped branch is
/// covered too), [`marginal`], [`posterior`] and [`rescale`] -- so the invariant holds for frozen
/// branches and every public aggregation, not only a branch actively being conditioned.
fn validate_branch_weight(b: &HistoryBranch, context: &str) {
    assert!(
        b.q.is_finite() && (0.0..=1.0).contains(&b.q),
        "{context}: branch {} weight q = {} is not a finite value in [0, 1]",
        b.id,
        b.q
    );
}

/// Asserts that seat entry `index` of branch `b` holds 1326 finite, non-negative masses, naming
/// the offending combo.
fn check_masses(b: &HistoryBranch, index: usize, context: &str) {
    let s = &b.seats[index];
    assert!(s.mass.len() == COMBOS, "{context}: branch {} seat {:?} has {} masses, expected {COMBOS}", b.id, s.seat, s.mass.len());
    for (c, w) in s.mass.iter().enumerate() {
        assert!(w.is_finite() && *w >= 0.0, "{context}: branch {} seat {:?} mass[{c}] = {w} is not a finite non-negative value", b.id, s.seat);
    }
}

/// The public marginal `r_S[c] = sum_k q_k * w_{S,k}[c]` of `seat` over every branch (residual
/// and stopped included); branches without the seat contribute nothing.
///
/// # Panics
/// Always, naming the offending branch or combo, if a branch weight is not a finite value in
/// `[0, 1]`, a branch holds the seat with other than 1326 finite non-negative masses, or a
/// positive branch weight and mass on some branch underflow to a zero combo marginal that cannot
/// be represented as a positive `f64` (there is no positive-reach threshold).
pub fn marginal(bs: &[HistoryBranch], seat: Seat) -> Vec<f64> {
    // R1/R2 fix: validate every branch's weight (including frozen ones) and its masses before
    // aggregating, and accumulate with the same scaled technique as `condition` so a representable
    // positive marginal is never lost to a per-branch underflow (see the module docs). Zero is
    // decided from the branch weight/mass operands, never from whether the accumulated sum happens
    // to compute to zero.
    let scale_half = pow2(350);
    let denom = scale_half * scale_half;
    let mut scaled = vec![0.0_f64; COMBOS];
    let mut positive = vec![false; COMBOS];
    for b in bs {
        validate_branch_weight(b, "marginal");
        if let Some(index) = b.seats.iter().position(|s| s.seat == seat) {
            check_masses(b, index, "marginal");
            let s = &b.seats[index];
            let q_scaled = b.q * scale_half;
            for (c, w) in s.mass.iter().enumerate() {
                if b.q > 0.0 && *w > 0.0 {
                    positive[c] = true;
                }
                scaled[c] += q_scaled * (w * scale_half);
            }
        }
    }
    scaled
        .into_iter()
        .zip(positive)
        .enumerate()
        .map(|(c, (v, pos))| {
            let out = v / denom;
            assert!(
                out > 0.0 || !pos,
                "marginal: seat {seat:?} combo {c} underflowed to 0 despite a positive branch weight and mass on some branch (the exact product is smaller than the smallest representable positive f64)"
            );
            out
        })
        .collect()
}

/// The branch posterior `pi_{S,k}[c] = q_k * w_{S,k}[c] / r_S[c]` of `seat` for combo `c`, one
/// entry per branch in list order (summing to 1 where `r_S[c] > 0`); all zeros where the combo
/// has no public mass.
///
/// # Panics
/// Always, if `c` is not a combo index, a branch weight is not a finite value in `[0, 1]`, or a
/// branch lacks the seat (both directly and through [`marginal`]'s own panics).
pub fn posterior(bs: &[HistoryBranch], seat: Seat, c: usize) -> Vec<f64> {
    assert!(c < COMBOS, "posterior: combo {c} is out of range 0..{COMBOS}");
    let r = marginal(bs, seat)[c];
    bs.iter()
        .map(|b| {
            validate_branch_weight(b, "posterior");
            if r == 0. {
                0.
            } else {
                let s = b.seats.iter().find(|s| s.seat == seat).unwrap_or_else(|| panic!("posterior: branch {} has no seat {seat:?}", b.id));
                b.q * s.mass[c] / r
            }
        })
        .collect()
}

/// The seat-common rescale of spec sections 8.4 and 9.2, run after every observed action: for
/// each seat `S`, one factor `m_S = max_c r_S[c]` divides that seat's masses in **every** branch
/// (residual and stopped included), and `ln(m_S)` is added to `log_reach[S]` (indexed by seat
/// id). `q` is never altered, so no posterior and no branch weight changes. A seat whose marginal
/// is zero everywhere is left untouched.
///
/// # Panics
/// Always, if the branches do not all hold the same seats in the same order, a branch weight is
/// not a finite value in `[0, 1]`, a marginal entry is negative or non-finite (a NaN would
/// otherwise be silently skipped by the maximum), `log_reach` has no entry for a seat, or the
/// rescaled marginal's maximum is not 1 within [`MASS_TOLERANCE`].
pub fn rescale(bs: &mut [HistoryBranch], logs: &mut [f64]) {
    let seats: Vec<Seat> = bs.first().map(|b| b.seats.iter().map(|s| s.seat).collect()).unwrap_or_default();
    for b in bs.iter() {
        let own: Vec<Seat> = b.seats.iter().map(|s| s.seat).collect();
        assert!(own == seats, "rescale: branch {} holds seats {own:?}, but the first branch holds {seats:?}", b.id);
        validate_branch_weight(b, "rescale");
    }
    for seat in seats {
        let r = marginal(bs, seat);
        for (c, x) in r.iter().enumerate() {
            assert!(x.is_finite() && *x >= 0.0, "rescale: seat {seat:?} marginal[{c}] = {x} is not a finite non-negative value");
        }
        let m = r.into_iter().fold(0.0_f64, f64::max);
        if m == 0. {
            continue;
        }
        let slot = seat.0 as usize;
        assert!(slot < logs.len(), "rescale: log_reach has {} entries, none for seat {seat:?}", logs.len());
        for b in bs.iter_mut() {
            for s in &mut b.seats {
                if s.seat == seat {
                    for (c, w) in s.mass.iter_mut().enumerate() {
                        let before = *w;
                        *w /= m;
                        assert!(
                            *w > 0.0 || before == 0.0,
                            "rescale: branch {} seat {seat:?} mass[{c}] = {before} underflowed to 0 when divided by {m}",
                            b.id
                        );
                    }
                }
            }
        }
        logs[slot] += m.ln();
        let peak = marginal(bs, seat).into_iter().fold(0.0_f64, f64::max);
        assert!(
            (peak - 1.0).abs() <= MASS_TOLERANCE && logs[slot].is_finite(),
            "rescale: seat {seat:?} rescaled by {m} has maximum marginal {peak} and log_reach {}",
            logs[slot]
        );
    }
}

/// Converts a rescaled `f64` marginal to the `f32` [`Range1326`] wire type (spec section 9.2) --
/// the replay's only narrowing, done after validation. Zero (either sign) stays `+0.0`; a
/// positive value below `f32::MIN_POSITIVE` becomes `f32::MIN_POSITIVE`, so positive reach stays
/// positive and the range hash of spec section 2 still sees a supported combo; every other value
/// rounds to the nearest `f32`.
///
/// # Panics
/// Always, naming the combo, if `r` does not have 1326 entries or any entry is non-finite,
/// negative, or above `1 + MASS_TOLERANCE` (a marginal not rescaled to maximum 1): out-of-domain
/// input is an error, never clamped into `[0, 1]`.
pub fn range_output(r: &[f64]) -> Range1326 {
    assert!(r.len() == COMBOS, "range_output: marginal has {} entries, expected {COMBOS}", r.len());
    for (c, x) in r.iter().enumerate() {
        assert!(
            x.is_finite() && *x >= 0.0 && *x <= 1.0 + MASS_TOLERANCE,
            "range_output: marginal[{c}] = {x} is not a finite weight in [0, 1]"
        );
    }
    let out = Range1326(std::array::from_fn(|i| if r[i] == 0. { 0. } else { r[i].max(f32::MIN_POSITIVE as f64) as f32 }));
    for (c, w) in out.0.iter().enumerate() {
        assert!(
            (*w == 0.0 && w.is_sign_positive() && r[c] == 0.0) || (*w >= f32::MIN_POSITIVE && *w <= 1.0),
            "range_output: marginal[{c}] = {} narrowed to {w}, outside the f32 output domain",
            r[c]
        );
    }
    out
}
