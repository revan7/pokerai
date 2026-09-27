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

use proto::{Action, ApproxReason, Range1326, Seat, COMBOS};

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

/// Caps the live branch list at 4, merging every branch beyond the four highest `q_k` (ties break
/// by ascending `id`, i.e. creation order) into the single frozen residual (spec sections 8.4 and
/// 9.2): `q_R' = q_R + sum_k q_k` and, per seat, `w_{S,R}'[c] = (q_R * w_{S,R}[c] + sum_k q_k *
/// w_{S,k}[c]) / q_R'`. This is exactly the weighted average that keeps every seat's public
/// marginal `r_S[c] = sum_k q_k * w_{S,k}[c]` unchanged: the merged terms are regrouped, never
/// dropped or renormalized away. It is evaluated with normalized weights `q_i / q_R'` (see
/// [`merge_into_residual`]), never through the products `q_i * w_i[c]`, so a combo whose only
/// support lies in the merged branches keeps a representable positive mass. A stopped branch
/// (spec section 9.3, `stopped: Some(reason)`) is not otherwise distinguished from a live one
/// here -- it keeps its rank by `q` and can itself be merged into the residual on a later
/// overflow, at which point its `stopped` reason is dropped (the residual carries no reason of its
/// own; [`residual_reason`] reports the cap, not why any one merged branch stopped). If no
/// residual exists yet, the first merged branch becomes it (its `translated` history, `stopped`
/// reason and every seat's `node` are cleared, since the residual has no history or node of its
/// own); every following overflow event folds into that same residual -- at most one residual
/// ever exists afterward. The output order is every surviving live branch by ascending `id`, then
/// the residual last (if any); `bs` is otherwise unchanged when there are 4 or fewer live branches
/// (nothing overflows).
///
/// # Panics
/// Always, if more than one input branch is already marked residual, any branch's weight is not a
/// finite value in `[0, 1]`, any seat's masses are not 1326 finite non-negative values, a merged
/// branch's seats do not line up with the residual's, a merged residual mass is not finite and
/// non-negative, a positively supported merged mass underflows to zero (there is no
/// positive-reach threshold, so an unrepresentable average is an error rather than a silent
/// zero), or the total `q` across every branch moves by more than [`MASS_TOLERANCE`] (relative)
/// across the cap -- the merge only ever regroups existing mass, never creates or drops it.
pub fn cap_branches(bs: &mut Vec<HistoryBranch>) {
    for b in bs.iter() {
        validate_branch_weight(b, "cap_branches");
        for index in 0..b.seats.len() {
            check_masses(b, index, "cap_branches");
        }
    }
    let residual_count = bs.iter().filter(|b| b.residual).count();
    assert!(residual_count <= 1, "cap_branches: {residual_count} branches are already marked residual, expected at most 1");
    let before: f64 = bs.iter().map(|b| b.q).sum();

    let mut live = Vec::new();
    let mut residual: Option<HistoryBranch> = None;
    for b in bs.drain(..) {
        if b.residual {
            residual = Some(b);
        } else {
            live.push(b);
        }
    }
    live.sort_by(|a, b| b.q.total_cmp(&a.q).then(a.id.cmp(&b.id)));
    let mut overflow = if live.len() > 4 { live.split_off(4) } else { Vec::new() }.into_iter();
    if residual.is_none() {
        // The first overflow branch (heaviest, then earliest) becomes the residual.
        residual = overflow.next().map(|mut r| {
            r.residual = true;
            r.translated.clear();
            r.stopped = None;
            for s in &mut r.seats {
                s.node = None;
            }
            r
        });
    }
    let merged: Vec<HistoryBranch> = overflow.collect();
    if let Some(r) = &mut residual {
        if !merged.is_empty() {
            merge_into_residual(r, &merged);
        }
    }
    live.sort_by_key(|b| b.id);
    bs.extend(live);
    if let Some(r) = residual {
        bs.push(r);
    }

    let after: f64 = bs.iter().map(|b| b.q).sum();
    assert!(
        (after - before).abs() <= MASS_TOLERANCE * before.max(1.0),
        "cap_branches: total branch weight moved from {before} to {after}, outside MASS_TOLERANCE"
    );
}

/// Folds every branch of `merged` into the residual `r` in one step (spec section 8.4):
/// `q_R' = q_R + sum_k q_k` and, per seat and combo, `w_R'[c] = sum_i a_i * w_i[c]` over
/// `i in {R} + merged`, with the **normalized weights** `a_i = q_i / q_R'`. Dividing the weights by
/// their total before multiplying by the masses means no `q_i * w_i[c]` product is ever formed: such
/// a product of a tiny weight and a mass can round to zero even when the weighted average itself is
/// representable, which would erase the only support of a combo (there is no positive-reach
/// threshold). The weights are carried with the module's power-of-two scale, `(q_i * 2^350) /
/// q_R'` (never subnormal for a positive `q_i <= q_R'`), and the scale is removed once, after the
/// sum -- the same technique as [`marginal`]'s scaled accumulation -- so every term keeps full
/// precision and only the final result rounds. If that scaled sum overflows (masses above about
/// `2^673`, far outside a rescaled replay), the unscaled normalized weights are used instead: at
/// that magnitude no term is anywhere near the underflow range. Whether a combo has support is
/// decided from the operands (`q_i > 0` and `w_i[c] > 0` for some `i`), never from the computed
/// sum, and a supported combo whose exact average is below the smallest positive `f64` is rejected
/// with a named assertion, as [`condition`] and [`marginal`] do.
///
/// When every weight is zero the average is undefined, and no term contributes to any marginal:
/// the residual keeps its own masses (the regroup is exact either way) and weight 0.
///
/// # Panics
/// Always, naming the seat and combo, if a merged branch's seats do not line up with the
/// residual's, a merged mass is not finite and non-negative, or a positively supported merged mass
/// underflows to zero.
fn merge_into_residual(r: &mut HistoryBranch, merged: &[HistoryBranch]) {
    for b in merged {
        assert_eq!(
            r.seats.len(),
            b.seats.len(),
            "cap_branches: residual holds {} seats, overflow branch {} holds {}",
            r.seats.len(),
            b.id,
            b.seats.len()
        );
        for (rs, ss) in r.seats.iter().zip(&b.seats) {
            assert_eq!(
                rs.seat, ss.seat,
                "cap_branches: residual seat {:?} does not line up with overflow branch {}'s seat {:?}",
                rs.seat, b.id, ss.seat
            );
        }
    }
    // Weight `i = 0` is the residual's own, `i >= 1` the merged branches' in rank order.
    let weights: Vec<f64> = std::iter::once(r.q).chain(merged.iter().map(|b| b.q)).collect();
    let total: f64 = weights.iter().sum();
    r.q = total;
    if total == 0.0 {
        return;
    }
    let scale = pow2(350);
    let normalized: Vec<f64> = weights.iter().map(|q| q / total).collect();
    let normalized_scaled: Vec<f64> = weights.iter().map(|q| q * scale / total).collect();
    for index in 0..r.seats.len() {
        let seat = r.seats[index].seat;
        for c in 0..COMBOS {
            let own = r.seats[index].mass[c];
            let masses = std::iter::once(own).chain(merged.iter().map(|b| b.seats[index].mass[c]));
            let (mut scaled, mut plain, mut supported) = (0.0_f64, 0.0_f64, false);
            for (i, w) in masses.enumerate() {
                if weights[i] > 0.0 && w > 0.0 {
                    supported = true;
                }
                scaled += normalized_scaled[i] * w;
                plain += normalized[i] * w;
            }
            let mass = if scaled.is_finite() { scaled / scale } else { plain };
            assert!(
                mass.is_finite() && mass >= 0.0,
                "cap_branches: residual seat {seat:?} mass[{c}] merged to {mass}, not a finite non-negative value"
            );
            assert!(
                mass > 0.0 || !supported,
                "cap_branches: residual seat {seat:?} mass[{c}] underflowed to 0 despite a positive branch weight and mass on some merged branch (the exact weighted average is smaller than the smallest representable positive f64)"
            );
            r.seats[index].mass[c] = mass;
        }
    }
}

/// The `ApproxReason::BranchResidual` this branch list's cap has earned, or `None` when it holds
/// no residual (never capped, or built without one). The share is recomputed from the branches'
/// current `q` every call -- never cached from the cap that created the residual -- so later
/// conditioning that changes live `q` (the residual itself is frozen and never conditioned; see
/// [`condition`]) is reflected immediately, as spec section 8.4 requires: "recompute the displayed
/// share after later evidence; storing only the original ... share would be wrong."
///
/// # Panics
/// Always, if any branch's weight is not a finite value in `[0, 1]`, or every branch's weight is
/// 0 (a residual's share of a zero total is undefined).
pub fn residual_reason(bs: &[HistoryBranch], hero: Seat) -> Option<ApproxReason> {
    for b in bs {
        validate_branch_weight(b, "residual_reason");
    }
    let r = bs.iter().find(|b| b.residual)?;
    let total: f64 = bs.iter().map(|b| b.q).sum();
    assert!(total > 0.0, "residual_reason: total branch weight is {total}, cannot compute a residual share");
    let pct = (100.0 * r.q / total) as f32;
    assert!(pct.is_finite() && pct >= 0.0, "residual_reason: residual share {pct} is not a finite non-negative percentage");
    Some(ApproxReason::BranchResidual { seat: hero, residual_mass_pct: pct, cause: "cap".into() })
}

/// Splits every **live** branch (a residual or a stopped branch is copied through unchanged, per
/// [`condition`]'s own frozen-branch rule) across one observed wager's common menu -- `(action, f,
/// p)` triples, applied as [`condition`]`(branch, actor, p, f)` -- preserving parent creation
/// order and, within one parent, choice order (child A before child B); a choice whose integrated
/// likelihood is 0 does not create a child. This is the test-only common-menu case: production
/// replay's branches can each carry a different menu (a different translated history reaching a
/// different node), so it builds each branch's own choices and calls [`condition`] directly,
/// running [`cap_branches`] once after every parent in the batch has been expanded -- never per
/// parent, or an earlier parent's overflow would compete against a later, still-unexpanded
/// parent's children for the same four live slots (see the module docs' transactional-update
/// section).
///
/// Every created child's `parent` is set to its parent's `id`, `split_by` to `actor`, and `action`
/// is appended to its `translated` history; every other field ([`condition`]'s own `q` and mass
/// update aside) is copied from the parent.
///
/// Ids keep creation order, which is the cap's tie-break (ties keep the earlier-created branch).
/// The input generation's ids must be distinct. New children are counted in a wide integer and
/// numbered after every input id, in creation order, so every new child sorts after every older
/// retained branch; nothing is narrowed to `u8` until the ids are final:
/// - If every child's id fits in a `u8` (at most `u8::MAX`), copied-through branches keep their
///   ids and `parent` references unchanged, and the children take the ids one past the highest
///   input id onward.
/// - Otherwise the ids are compacted. A collision-free old-to-new map is built from the whole input
///   generation first -- its ids in ascending (creation) order become `0, 1, 2, ...`, including
///   the parents consumed by this split -- and the children take the ids after it. Every `parent`
///   reference is remapped through that map, never through output ids: a child names its
///   consumed parent's mapped id, a slot that no output branch holds, so no branch becomes its
///   own parent and no child names a sibling, a cousin or a frozen branch; a reference to a branch
///   outside the input generation (an ancestor that no longer exists) becomes `None` rather than
///   resolving to an unrelated output branch.
///
/// A parent all of whose children have `M = 0` (an impossible action against that parent's public
/// range) contributes nothing to `out`; a parent whose *every* child across the whole `bs` is
/// dropped this way is the zero-support case, handled by the caller (this batch-transactional
/// function does not itself reject an update -- spec section 9.2's zero-support rejection keeps
/// the pre-action list, which is `bs` unchanged, not an empty `out`).
///
/// # Panics
/// Always, if two input branches share an id, if a compacted input generation and its new
/// children together exceed the 256 ids a `u8` can address, or through [`condition`]'s own
/// panics.
pub fn split_action(bs: &[HistoryBranch], actor: Seat, choices: &[(Action, f64, Vec<f64>)]) -> Vec<HistoryBranch> {
    // The input generation's ids in creation order; distinct, so the old-to-new map is collision-free.
    let mut generation: Vec<u8> = bs.iter().map(|b| b.id).collect();
    generation.sort_unstable();
    for pair in generation.windows(2) {
        assert!(pair[0] != pair[1], "split_action: branch id {} appears more than once in the input generation", pair[0]);
    }

    let mut out: Vec<HistoryBranch> = Vec::new();
    // Per output entry: `Some(k)` for the `k`-th new child in creation order, `None` for a branch
    // copied through unchanged. `k` is a wide counter; nothing is narrowed until the ids are final.
    let mut child_rank: Vec<Option<usize>> = Vec::new();
    let mut children = 0_usize;
    for b in bs {
        if b.residual || b.stopped.is_some() {
            out.push(b.clone());
            child_rank.push(None);
            continue;
        }
        for (action, f, p) in choices {
            let Some(mut child) = condition(b, actor, p, *f) else { continue };
            child.parent = Some(b.id);
            child.split_by = Some(actor);
            child.translated.push((actor, action.clone()));
            out.push(child);
            child_rank.push(Some(children));
            children += 1;
        }
    }

    let id_space = usize::from(u8::MAX) + 1;
    let first_child = generation.last().map_or(0, |&id| usize::from(id) + 1);
    if first_child + children <= id_space {
        for (b, rank) in out.iter_mut().zip(&child_rank) {
            if let Some(k) = rank {
                b.id = narrow_id(first_child + k);
            }
        }
    } else {
        let base = generation.len();
        assert!(
            base + children <= id_space,
            "split_action: {base} input branches and {children} new children cannot be addressed by a u8 id"
        );
        let mapped = |old: u8| generation.binary_search(&old).ok();
        for (b, rank) in out.iter_mut().zip(&child_rank) {
            let wide = match rank {
                Some(k) => base + k,
                None => mapped(b.id).expect("a copied-through branch belongs to the input generation"),
            };
            b.parent = b.parent.and_then(mapped).map(narrow_id);
            b.id = narrow_id(wide);
        }
    }
    out
}

/// Narrows a final wide branch id to the public `u8` field.
///
/// # Panics
/// Always, if `wide` does not fit in a `u8` (the callers check the id space before narrowing).
fn narrow_id(wide: usize) -> u8 {
    u8::try_from(wide).unwrap_or_else(|_| panic!("split_action: branch id {wide} does not fit in a u8"))
}
