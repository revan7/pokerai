//! Wager interpolation and legality-after-mapping (spec section 8.4), the last chip-domain step
//! before a translated preflop node can be used as replay evidence or shown as advice: an observed
//! wager is converted to a pot fraction and interpolated against a node's own menu sizes (the
//! pseudo-harmonic split), and a chip-domain `ExpandedNode` (P3.T9) menu is remapped onto the live
//! tree's actual [`proto::LegalAction`]s so every row's output menu is identical and no probability
//! mass is ever silently dropped or guessed as a fold.
//!
//! This module owns two independent halves, per the task brief: [`interpolate`]/[`wager_fraction`]/
//! [`menu_fractions`] (section 8.4's likelihood-interpolation formula, used once per **observed
//! action** at replay time -- section 9.2's off-menu branch), and [`destination_map`]/
//! [`legalize_row`] (section 8.4's legality-after-mapping rule, used once per **node**: the
//! destination map is built a single time and then walked, unchanged, for all 1326 combo rows).
//!
//! The last section (P3.T16) assembles hero's current decision from those mapped nodes over the
//! shared history branches: [`mix_action`] is section 8.4's per-action kernel (known frequency,
//! EV only over complete branch support) and [`mix_nodes`] builds the whole [`MixedNode`] --
//! per-combo advice, the unresolved posterior mass, the range-level mix and the reasons.

use crate::branches::{posterior, HistoryBranch, MASS_TOLERANCE};
use crate::envelope::{EvReference, SourceKind};
use crate::ev::ExpandedNode;
use proto::{Action, ActionAdvice, ApproxReason, LegalAction, Seat, Unavailable, UnsupportedReason, COMBOS};

// ---------------------------------------------------------------------------------------------
// Likelihood interpolation (spec section 8.4).
// ---------------------------------------------------------------------------------------------

/// One observed wager's pot fraction interpolated against a node's own menu sizes: the
/// pseudo-harmonic split between the two adjacent sizes that bracket it, or a clamp to the
/// nearest size at either edge of the menu.
///
/// `choices` is one or two `(menu index, frequency)` pairs (frequencies sum to `1.0`); `deviation`
/// is `min(|s - A|, |s - B|)` over the size(s) actually used, in pot-fraction units (spec section
/// 8.4's `d`); `clamped` is **not** a note about `s` or the menu being sanitized -- both are
/// validated (and rejected as `None`) before any of this runs -- it names the destination-mapping
/// rule of spec section 8.4: a single menu size, two equal-sized menu entries, an observed size
/// below the smallest menu size, or above the largest when no all-in menu action is present, are
/// all recorded as a size-1 clamp rather than a two-way split.
#[derive(Clone, Debug, PartialEq)]
pub struct Interpolation {
    pub choices: Vec<(usize, f64)>,
    pub deviation: f64,
    pub clamped: bool,
}

/// Spec section 8.4's pseudo-harmonic split of an observed pot-fraction wager `s` against a
/// node's own menu sizes `menu` (each a `(index, pot fraction)` pair, e.g. from
/// [`menu_fractions`]): `f_A = (B - s)(1 + A) / ((B - A)(1 + s))`, `f_B = 1 - f_A`, so that
/// `P(obs | combo) = f_A * P(A | combo) + f_B * P(B | combo)`.
///
/// Any numeric INPUT outside its domain -- a negative or non-finite `s`, an empty menu, or a
/// negative or non-finite menu size -- is an error, checked in `f64` (no narrowing ever happens
/// in this function), returning `None` rather than a fabricated `Interpolation` (standing ruling:
/// input is validated wide, before use, never clamped into range). Boundaries, per spec section
/// 8.4: a single menu size, or two menu sizes that dedup to one (`A == B`), clamps with `f = 1`;
/// below the smallest size clamps to it; above the largest (there being no larger size to
/// interpolate against -- an all-in size on the menu is just another size here, so "the largest"
/// already accounts for it) clamps to it too. An exact match to a menu size short-circuits to that
/// size alone, deviation `0.0`, never treated as a clamp.
///
/// Every `Some` result is a probability split: each weight finite and in `[0, 1]`, the weights
/// summing to 1, for every accepted finite input however large or close together the sizes are
/// (fix round 1, review R6: the formula is evaluated as two bounded ratios, never as two products
/// that can overflow). A split that still cannot be represented returns `None`, never NaN weights.
pub fn interpolate(s: f64, menu: &[(usize, f64)]) -> Option<Interpolation> {
    if !s.is_finite() || s < 0.0 || menu.is_empty() {
        return None;
    }
    let mut sizes = menu.to_vec();
    sizes.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    sizes.dedup_by(|a, b| a.1 == b.1);
    if sizes.iter().any(|(_, x)| !x.is_finite() || *x < 0.0) {
        return None;
    }
    if let Some(&(i, _)) = sizes.iter().find(|(_, x)| *x == s) {
        return Some(Interpolation { choices: vec![(i, 1.0)], deviation: 0.0, clamped: false });
    }
    let first = sizes[0];
    let last = *sizes.last()?;
    if sizes.len() == 1 || s < first.1 || s > last.1 {
        let (i, x) = if s < first.1 { first } else { last };
        return Some(Interpolation { choices: vec![(i, 1.0)], deviation: (s - x).abs(), clamped: true });
    }
    let pair = sizes.windows(2).find(|p| p[0].1 <= s && s <= p[1].1)?;
    let ((i, a), (j, b)) = (pair[0], pair[1]);
    // R6 (fix round 1): the same formula, evaluated as two bounded ratios instead of two products.
    // The product form `(B - s)(1 + A) / ((B - A)(1 + s))` overflows both products to infinity for
    // large finite sizes (1.5e200 between 1e200 and 2e200) and returns NaN weights. Here the
    // bracket guarantees `A < s < B` (equal sizes were deduplicated, an exact hit returned above),
    // so `B - A > 0` even across a subnormal gap, `0 <= (B - s) / (B - A) <= 1` and
    // `0 < (1 + A) / (1 + s) <= 1`: every operand stays finite and `f_A` lands in `[0, 1]`. The
    // final check is the guard for any case that is still not representable: `None`, never a
    // NaN/out-of-range weight and never a clamped one.
    let fa = ((b - s) / (b - a)) * ((1.0 + a) / (1.0 + s));
    if !fa.is_finite() || !(0.0..=1.0).contains(&fa) {
        return None;
    }
    Some(Interpolation { choices: vec![(i, fa), (j, 1.0 - fa)], deviation: (s - a).abs().min((s - b).abs()), clamped: false })
}

/// The observed wager's pot fraction at the parent node (spec section 8.4): `s = (chips added
/// beyond a call) / (pot after the bettor's call)`. `own` is the actor's own contribution this
/// street before this wager; `call` is what the actor owed to match the current wager (`0` for an
/// opening bet); `pot` is the pot **before** this actor's call is added, reconstructed from the
/// financial state at the mapped parent node -- never a later observed pot. For a raise-to `to`,
/// this is `(to - own - call) / (pot + call)`. All arithmetic is in `f64` on the widened `u32`
/// operands, so no intermediate can overflow.
///
/// Returns `None` -- never a panic, never a clamped value -- when the inputs have no pot fraction:
/// `pot + call == 0` (nothing to take a fraction of), `to < own + call` (not a wager at all), or a
/// non-finite result. These are **recoverable translation-domain mismatches**, not broken
/// invariants: a legal observed history can reach them at a mapped parent whose money follows the
/// translated source history while `to` is the actual observed amount (e.g. an observed open to 7
/// split onto a source open to 20, where the next actor's actual legal re-raise to 12 is below its
/// mapped call of 20). The caller (P3.T13's replay) takes its unmappable-branch path on `None`.
///
/// Plan-mandated deviation (fix round 1, review R4): the brief's signature was `-> f64`; the
/// orchestrator ruled it fallible so the mismatch above is not a panic.
pub fn wager_fraction(to: u32, own: u32, call: u32, pot: u32) -> Option<f64> {
    let denom = f64::from(pot) + f64::from(call);
    let numer = f64::from(to) - f64::from(own) - f64::from(call);
    if denom <= 0.0 || numer < 0.0 {
        return None;
    }
    let s = numer / denom;
    s.is_finite().then_some(s)
}

/// A source node's own wager-sized actions (`Bet`/`Raise`/`AllIn`), converted to the
/// `(menu index, pot fraction)` pairs [`interpolate`]'s `menu` parameter expects, at one parent
/// node's financial context (`own`/`call`/`pot`, the same meaning as [`wager_fraction`]'s). `Fold`,
/// `Check` and `Call` carry no size and are skipped; the returned index is that action's own
/// position within `actions`, so a caller can look the chosen menu action back up directly.
///
/// Returns `None` when **any** wager-sized action has no pot fraction at this parent
/// ([`wager_fraction`] is `None`) -- typically a source `AllIn`, which carries the actor's ACTUAL
/// maximum, falling below a mapped call taken from the translated source history. The whole
/// conversion fails rather than dropping that size, because a menu missing one of its sizes would
/// silently change the interpolation bracket; the caller takes the same unmappable-branch path as
/// for a `None` observed fraction. A menu with no wager-sized action is `Some(vec![])`.
pub fn menu_fractions(actions: &[Action], own: u32, call: u32, pot: u32) -> Option<Vec<(usize, f64)>> {
    actions
        .iter()
        .enumerate()
        .filter_map(|(i, a)| match a {
            Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => Some(wager_fraction(*to, own, call, pot).map(|s| (i, s))),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Legality after mapping (spec section 8.4): one destination map per node, walked once per row.
// ---------------------------------------------------------------------------------------------

/// Where one source action's probability mass lands in the legalized output menu, with the
/// provenance [`legalize_row`] needs to decide EV ownership and to disclose the move (fix round 1,
/// review R1-R3):
///
/// - `index`: the destination's position in the output menu.
/// - `created`: the destination is **not** one of the original legal source actions -- no source
///   action that was legal as declared maps to it, so it is on the menu only because illegal
///   sources were moved there. Decided once, from pass 1's legal-source membership, never from
///   the menu as it grows during pass 2: a second move onto a destination a first move created
///   leaves it created.
/// - `source`: the ORIGINAL source action (chip domain, as declared), before any move.
/// - `moved`: `source` was illegal at the live node and its mass was moved to a different action
///   (`menu[index] != source`). A moved source never supplies its destination's EV; an unmoved
///   source is an **owner** of its destination (several unmoved sources co-own one destination
///   when their amounts round to the same chip action).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Destination {
    pub index: usize,
    pub created: bool,
    pub source: Action,
    pub moved: bool,
}

/// One combo row's chip-valued advice after legalization: `menu.len()` entries, in the same
/// deterministic order for every row of a node.
#[derive(Clone, Debug, PartialEq)]
pub struct MappedAction {
    pub action: Action,
    pub probability: f32,
    pub ev_chips: Option<f32>,
    pub unavailable: Option<Unavailable>,
}

/// [`legalize_row`]'s result: the legalized actions, human-readable notes about any move or
/// collision, and -- only when the whole node's [`destination_map`] itself failed -- the reason,
/// with no actions reported at all (spec section 8.4: a node that has no legal destination for
/// some observed history is unsupported for every row alike, never guessed action by action).
#[derive(Clone, Debug, PartialEq)]
pub struct MappedAdvice {
    pub actions: Vec<MappedAction>,
    pub notes: Vec<String>,
    pub unsupported: Option<UnsupportedReason>,
}

/// Whether source action `a` is itself one of the live tree's legal actions: a `Bet`/`Raise`
/// matches only its own kind's interval (an open `Bet` is never accepted as a `Raise`, and vice
/// versa), an `AllIn` matches only the exact chip amount the live tree allows, `Call`/`Check`/
/// `Fold` match their like-named legal entry regardless of any carried amount.
fn is_legal(a: &Action, legal: &[LegalAction]) -> bool {
    legal.iter().any(|la| match (a, la) {
        (Action::Fold, LegalAction::Fold) => true,
        (Action::Check, LegalAction::Check) => true,
        (Action::Call, LegalAction::Call { .. }) => true,
        (Action::Bet { to }, LegalAction::Bet { min_to, max_to }) => *to >= *min_to && *to <= *max_to,
        (Action::Raise { to }, LegalAction::Raise { min_to, max_to }) => *to >= *min_to && *to <= *max_to,
        (Action::AllIn { to }, LegalAction::AllIn { to: t }) => *to == *t,
        _ => false,
    })
}

/// The live tree's legal `Bet`/`Raise` interval, if either is legal at this node (the two are
/// mutually exclusive: an opening action offers at most a `Bet`, a facing wager offers at most a
/// `Raise`).
fn wager_interval(legal: &[LegalAction]) -> Option<(u32, u32)> {
    legal.iter().find_map(|la| match la {
        LegalAction::Bet { min_to, max_to } | LegalAction::Raise { min_to, max_to } => Some((*min_to, *max_to)),
        _ => None,
    })
}

/// The live tree's all-in chip amount, if going all-in is legal at this node.
fn allin_to(legal: &[LegalAction]) -> Option<u32> {
    legal.iter().find_map(|la| match la { LegalAction::AllIn { to } => Some(*to), _ => None })
}

/// Whether chip amount `to` falls short of the smallest legal (re)raise. When the live tree still
/// offers a partial `Bet`/`Raise` interval, that is its `min_to`. When the stack is too short for
/// any partial raise at all (only `AllIn` is legal), the smallest -- and only -- amount that can
/// legally increase the bet is the all-in amount itself, so anything short of it is "below the
/// minimum" too, and anything reaching it is not (see [`above_max`]'s matching half).
fn below_min(to: u32, legal: &[LegalAction]) -> bool {
    match wager_interval(legal) {
        Some((min_to, _)) => to < min_to,
        None => allin_to(legal).is_some_and(|max| to < max),
    }
}

/// Whether chip amount `to` exceeds the largest legal (re)raise. Mirrors [`below_min`]: with a
/// legal `Bet`/`Raise` interval, that is its `max_to`, strictly exceeded. With no such interval
/// (only `AllIn` legal), reaching **or** exceeding the all-in amount both resolve to the same
/// single legal wager, so the boundary itself belongs here, not to `below_min` -- together the two
/// predicates partition every `u32` with no gap and no overlap.
fn above_max(to: u32, legal: &[LegalAction]) -> bool {
    match wager_interval(legal) {
        Some((_, max_to)) => to > max_to,
        None => allin_to(legal).is_some_and(|max| to >= max),
    }
}

/// Among the source's own declared actions, the smallest `Raise` that is itself legal -- the
/// fallback destination for a below-minimum raise (spec section 8.4); `None` if no source `Raise`
/// is legal at all.
fn smallest_legal_menu_raise(actions: &[Action], legal: &[LegalAction]) -> Option<Action> {
    actions
        .iter()
        .filter(|a| matches!(a, Action::Raise { .. }) && is_legal(a, legal))
        .min_by_key(|a| match a { Action::Raise { to } => *to, _ => unreachable!("filtered to Raise above") })
        .copied()
}

/// `Action::Call`, if calling is legal at this node.
fn call_action(legal: &[LegalAction]) -> Option<Action> {
    legal.iter().any(|la| matches!(la, LegalAction::Call { .. })).then_some(Action::Call)
}

/// `Action::AllIn` at the live tree's all-in amount, if going all-in is legal at this node.
fn allin_action(legal: &[LegalAction]) -> Option<Action> {
    allin_to(legal).map(|to| Action::AllIn { to })
}

/// The canonical menu order of spec section 8.4 (`Fold`, `Check`, `Call`, wagers ascending by
/// `to`, `AllIn`), shared by [`destination_map`]'s per-node menu and [`mix_nodes`]'s union menu so
/// the two can never disagree.
fn menu_rank(a: &Action) -> (u8, u32) {
    match a {
        Action::Fold => (0, 0),
        Action::Check => (1, 0),
        Action::Call => (2, 0),
        Action::Bet { to } | Action::Raise { to } => (3, *to),
        Action::AllIn { to } => (4, *to),
    }
}

/// Sorts `menu` into the canonical order ([`menu_rank`]) and rewrites `created`/`mapped` to the
/// same permutation, so every row of a node sees an identical menu order regardless of the order
/// source actions happened to arrive in.
fn order_menu(menu: &mut Vec<Action>, created: &mut Vec<bool>, mapped: &mut [Option<usize>]) {
    let mut order: Vec<usize> = (0..menu.len()).collect();
    order.sort_by_key(|&i| menu_rank(&menu[i]));
    let mut old_to_new = vec![0usize; menu.len()];
    for (new_i, &old_i) in order.iter().enumerate() {
        old_to_new[old_i] = new_i;
    }
    let new_menu: Vec<Action> = order.iter().map(|&i| menu[i]).collect();
    let new_created: Vec<bool> = order.iter().map(|&i| created[i]).collect();
    *menu = new_menu;
    *created = new_created;
    for m in mapped.iter_mut() {
        if let Some(i) = m {
            *i = old_to_new[*i];
        }
    }
}

/// Builds the single destination map for one node (spec section 8.4's legality-after-mapping
/// rule): every one of the source's own declared `actions` is matched against the live tree's
/// `legal` actions, producing the deterministic output menu (Fold, Check, Call, wagers ascending,
/// AllIn) plus, for each source action, a [`Destination`]: where its probability mass lands,
/// whether that destination exists only because of moves, the original source action, and whether
/// it was moved (which decides EV ownership in [`legalize_row`]). Built once per node and
/// reused for every one of the 1326 combo rows via [`legalize_row`] -- a single per-combo
/// `legalize` could not express this shared map, which is why the two halves are separate
/// functions.
///
/// Pass 1 gives every source action that is itself legal its own destination, and is the only
/// pass that establishes ownership: the menu entries it creates are exactly the original legal
/// source actions. Pass 2 moves every remaining (illegal) source action by kind: a below-minimum
/// `Raise` goes to the smallest legal source-menu raise, or to `Call` if none is legal; a `Bet` or
/// `Raise` above the legal maximum goes to the legal `AllIn`; anything else (including a
/// below-minimum `Bet`, which section 8.4 gives no fallback rule for) has no legal destination at
/// all and is rejected to the caller as `UnsupportedReason::UnsupportedHistory` -- never guessed
/// as a fold or silently dropped. A destination first put on the menu by pass 2 is `created` and
/// stays created however many further moves land on it (fix round 1, review R2).
pub fn destination_map(actions: &[Action], legal: &[LegalAction]) -> Result<(Vec<Action>, Vec<Destination>), UnsupportedReason> {
    fn position_or_push(menu: &mut Vec<Action>, a: Action) -> usize {
        match menu.iter().position(|x| *x == a) {
            Some(i) => i,
            None => {
                menu.push(a);
                menu.len() - 1
            }
        }
    }
    let mut menu: Vec<Action> = Vec::new();
    let mut mapped: Vec<Option<usize>> = vec![None; actions.len()];
    // Pass 1: every source action that is itself legal owns its destination (identical rounded
    // legal sources share one entry and co-own it).
    let legal_source: Vec<bool> = actions.iter().map(|a| is_legal(a, legal)).collect();
    for (k, a) in actions.iter().enumerate() {
        if legal_source[k] {
            mapped[k] = Some(position_or_push(&mut menu, *a));
        }
    }
    // Every entry pushed from here on exists only because of a move.
    let owned_entries = menu.len();
    // Pass 2: illegal source actions move by kind (spec section 8.4's legality-after-mapping rule).
    for (k, a) in actions.iter().enumerate() {
        if legal_source[k] {
            continue;
        }
        let target = match a {
            Action::Raise { to } if below_min(*to, legal) => smallest_legal_menu_raise(actions, legal).or_else(|| call_action(legal)),
            Action::Bet { to } | Action::Raise { to } if above_max(*to, legal) => allin_action(legal),
            _ => None,
        };
        let Some(t) = target else {
            return Err(UnsupportedReason::UnsupportedHistory { reason: format!("no legal destination for {a:?}") });
        };
        mapped[k] = Some(position_or_push(&mut menu, t));
    }
    let mut created: Vec<bool> = (0..menu.len()).map(|i| i >= owned_entries).collect();
    order_menu(&mut menu, &mut created, &mut mapped);
    let map = mapped
        .into_iter()
        .zip(actions)
        .zip(legal_source)
        .enumerate()
        .map(|(k, ((i, a), was_legal))| {
            let i = i.unwrap_or_else(|| panic!("destination_map: source action {k} ({a:?}) was never mapped"));
            Destination { index: i, created: created[i], source: *a, moved: !was_legal }
        })
        .collect();
    Ok((menu, map))
}

/// Walks one combo's probability/EV row through a node's [`destination_map`] output exactly once,
/// merging mass onto the legalized menu and deciding each destination's EV (spec section 8.4;
/// ownership per fix round 1, review R1-R3):
///
/// - **Probability**: each destination's probability is the plain sum of every source probability
///   mapped to it, moved or not. Nothing is normalized, so output mass equals input mass.
/// - **EV ownership**: only the destination's **owners** -- the unmoved sources, i.e. the legal
///   source actions that map to themselves -- can supply its EV. A moved source's EV is never
///   used, whatever the source order. With one owner, the destination keeps exactly that owner's
///   EV (`None` stays `None`: a missing EV is never borrowed from a moved source). With several
///   co-owners (identical rounded actions), the EV is kept only when every co-owner has the same
///   EV; otherwise there is no unique payoff and it is omitted, with a collision note, rather than
///   invented. (Preferring an independently exactly represented co-owner would need the
///   pre-rounding source sizes, which the chip-domain `actions` no longer carry; omission is the
///   conservative rule.)
/// - **Created destinations** (no owner) get `ev_chips: None` and `unavailable:
///   MovedProbability { from }`, where `from` is the ORIGINAL action of the first moved source in
///   source order.
/// - **Notes**: one note per actual move, naming the original source action, its probability and
///   its destination; one note per destination with several co-owners. A source that maps to
///   itself gets no note.
///
/// **Row preconditions.** `probs` and `evs` are one combo's row, indexed like the source actions
/// the map was built from (so both have `map.len()` entries). A reachable row is a probability
/// distribution over those actions; an explicitly unreachable row (`ExpandedNode::available` is
/// `false`) is all zeros and stays all zeros here -- no mass is ever invented for it -- and a row
/// that does not sum to 1 is never normalized or repaired: the caller consults `available` before
/// reading the output as a distribution. (The one normalization of an admitted row's source
/// rounding happens later, once, at [`mix_nodes`]'s mapped-node boundary -- its step 0.)
///
/// **Notes buffer.** The whole `notes` buffer -- including anything the caller had already put in
/// it -- is moved into the returned [`MappedAdvice::notes`], followed by this row's own notes, and
/// `notes` is left empty. A caller reusing one buffer across rows must re-add node-level notes
/// before each row, or keep them separately.
///
/// # Panics
/// Always (not only in debug builds; standing ruling (b)), naming the offending index or length,
/// if the row/map/menu relationship is malformed -- `probs`/`evs` not of length `map.len()`, a
/// destination index outside the menu, a menu entry that no source maps to or that appears twice,
/// a `moved`/`created` flag inconsistent with the menu -- or if a probability is not a finite
/// value in `[0, 1]` or an EV is not finite. These are broken invariants of a map built by
/// [`destination_map`] and a validated [`crate::ExpandedNode`] row, never recoverable input, so a
/// malformed row is rejected rather than truncated, padded or normalized.
pub fn legalize_row(menu: &[Action], map: &[Destination], probs: &[f32], evs: &[Option<f32>], notes: &mut Vec<String>) -> MappedAdvice {
    check_row(menu, map, probs, evs);
    let mut out: Vec<MappedAction> =
        menu.iter().map(|a| MappedAction { action: *a, probability: 0.0, ev_chips: None, unavailable: None }).collect();
    for (k, d) in map.iter().enumerate() {
        if d.moved {
            // `check_row` has established that `d.created` means "no unmoved source owns this".
            merge_probability(&mut out[d.index], &d.source, probs[k], d.created, notes);
        } else {
            out[d.index].probability += probs[k];
        }
    }
    for (i, dest) in out.iter_mut().enumerate() {
        let owners: Vec<usize> = (0..map.len()).filter(|&k| map[k].index == i && !map[k].moved).collect();
        let Some(&first) = owners.first() else {
            continue; // created: `merge_probability` already recorded `MovedProbability`, no EV
        };
        let ev = evs[first];
        let unique = ev.is_some() && owners.iter().all(|&k| evs[k] == ev);
        if unique {
            dest.ev_chips = ev;
        }
        if owners.len() > 1 {
            let shares: Vec<f32> = owners.iter().map(|&k| probs[k]).collect();
            notes.push(format!(
                "{} source actions collide on {:?} (source indices {owners:?}, probabilities {shares:?}); {}",
                owners.len(),
                menu[i],
                if unique { "their identical EV is kept" } else { "EV omitted: no unique payoff" }
            ));
        }
    }
    MappedAdvice { actions: out, notes: std::mem::take(notes), unsupported: None }
}

/// [`legalize_row`]'s always-on precondition checks (fix round 1, review R5): shape first (lengths,
/// indices), then the map/menu relationship, then the value domains, each naming what is wrong.
fn check_row(menu: &[Action], map: &[Destination], probs: &[f32], evs: &[Option<f32>]) {
    assert!(probs.len() == map.len(), "legalize_row: probs has length {} but the destination map has length {}", probs.len(), map.len());
    assert!(evs.len() == map.len(), "legalize_row: evs has length {} but the destination map has length {}", evs.len(), map.len());
    for (k, d) in map.iter().enumerate() {
        assert!(d.index < menu.len(), "legalize_row: map[{k}].index {} is out of range for a {}-action menu", d.index, menu.len());
    }
    for (i, a) in menu.iter().enumerate() {
        if let Some(j) = (i + 1..menu.len()).find(|&j| menu[j] == *a) {
            panic!("legalize_row: menu[{i}] and menu[{j}] are both {a:?}");
        }
        assert!(map.iter().any(|d| d.index == i), "legalize_row: menu[{i}] {a:?} has no source action mapped to it");
    }
    for (k, d) in map.iter().enumerate() {
        let target = &menu[d.index];
        assert!(
            d.moved == (d.source != *target),
            "legalize_row: map[{k}] source {:?} -> menu[{}] {target:?} has moved = {} (a source is moved exactly when it differs from its destination)",
            d.source,
            d.index,
            d.moved
        );
        let owned = map.iter().any(|e| e.index == d.index && !e.moved);
        assert!(
            d.created != owned,
            "legalize_row: map[{k}].created is {} but menu[{}] {target:?} {} (created means no unmoved source maps to it)",
            d.created,
            d.index,
            if owned { "has an unmoved source" } else { "has no unmoved source" }
        );
    }
    for (k, &p) in probs.iter().enumerate() {
        assert!(p.is_finite() && (0.0..=1.0).contains(&p), "legalize_row: probs[{k}] = {p} is not a probability in [0, 1]");
    }
    for (k, ev) in evs.iter().enumerate() {
        if let Some(v) = ev {
            assert!(v.is_finite(), "legalize_row: evs[{k}] = {v} is not finite");
        }
    }
}

/// Merges one **moved** source action's probability mass onto its legalized `destination` and
/// records the move note (the original action `from`, its probability, the destination). When the
/// destination is `created` (no legal source owns it), it gets no EV and is flagged
/// `Unavailable::MovedProbability { from }` -- by the first moved source merged into it, so a
/// caller that merges in source order names the first moved source.
///
/// # Panics
/// Always (not only in debug builds), if `from` is the destination's own action: a source that
/// maps to itself was not moved and must not be recorded as a move.
pub fn merge_probability(destination: &mut MappedAction, from: &Action, probability: f32, created: bool, notes: &mut Vec<String>) {
    assert!(*from != destination.action, "merge_probability: {from:?} maps to itself, which is not a move");
    destination.probability += probability;
    if created {
        destination.ev_chips = None;
        if destination.unavailable.is_none() {
            destination.unavailable = Some(Unavailable::MovedProbability { from: *from });
        }
    }
    notes.push(format!("Moved {from:?} probability {probability} to {:?}", destination.action));
}

// ---------------------------------------------------------------------------------------------
// Branch-supported assembly (spec section 8.4's node translation and assembly; sections 6, 8.3).
// ---------------------------------------------------------------------------------------------

/// Spec section 8.2's per-class sibling-sum admission tolerance: a reachable source row sums to
/// `1 +- 1e-3` (the bound `validate`'s wide gate applies to the wire values at load).
const SOURCE_ROW_TOLERANCE: f64 = 1e-3;

/// The `f32` carrier's rounding headroom on top of [`SOURCE_ROW_TOLERANCE`] at the mapped-node
/// boundary (fix round 1, T16-R1). Spec 8.2 admits a row on its wide (`f64`) wire values; the
/// mapped node carries those weights narrowed to `f32` (each off by at most `2^-24` of itself) and
/// merged by [`legalize_row`] in `f32` (each merge off by at most `2^-24` of the running row
/// mass), so an admitted `n`-action row can arrive up to about `n * 2^-24 * 1.001` outside
/// `1 +- 1e-3` -- two `0.5005` weights (wide sum 1.001, admitted) arrive as 1.0010000467. `1e-5`
/// covers any row of up to 150 source actions, far above any preflop menu, so no admitted row is
/// rejected here; it is a hundredth of the tolerance itself, so nothing materially outside
/// `1 +- 1e-3` is admitted.
const F32_CARRIER_SLACK: f64 = 1e-5;

/// Hero's current preflop node in one history branch, as the current decision's lookup in that
/// branch found it (spec section 8.4).
///
/// - `branch_id`: the [`HistoryBranch::id`] this entry belongs to.
/// - `node`: the source node expanded to chips (P3.T9) and already mapped onto the live legal menu
///   ([`destination_map`]/[`legalize_row`], P3.T10), so every action is a live chip action and no
///   action appears twice; `None` when the branch's key has no node.
/// - `key`: the key the lookup used, retained for diagnostics whether or not a node was found; it
///   names the missing node in `MissingPreflopNode` and in `BranchResidual`'s cause.
/// - `created`: the destinations on `node`'s menu that the legality-after-mapping rule **created**
///   (no legal source action owns them), each with the ORIGINAL source action whose probability
///   was moved there first -- `(destination, from)`, exactly the `MovedProbability { from }` that
///   [`legalize_row`] records on that destination. An [`ExpandedNode`] carries no per-action
///   `unavailable`, so without this list the assembly could not tell a created destination's
///   missing EV from an unnormalizable one. Empty when nothing was created. (Plan-3 deviation:
///   the brief's `BranchNode` has only the first three fields; `Default` lets a caller write
///   `BranchNode { branch_id, node, key, ..Default::default() }`.)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BranchNode {
    pub branch_id: u8,
    pub node: Option<ExpandedNode>,
    pub key: String,
    pub created: Vec<(Action, Action)>,
}

/// Hero's current preflop decision assembled over the shared history branches (spec sections 4.4
/// and 8.4).
///
/// - `actions`: one entry per action of the union of the mapped menus, in the canonical order
///   (Fold, Check, Call, wagers ascending, AllIn). `frequency` is the known mass
///   `sum_{k has a} pi_{H,k}[c] * P_k(a | c)`; `ev_bb` is the posterior-averaged EV only over
///   complete branch support, converted from chips to bb once; `headline` is always `false` (the
///   headline is Plan 2's `assemble::headline`, which reads `unresolved_mass`).
/// - `unresolved_mass`: hero's combo posterior mass on branches with no node (the residual, a
///   stopped branch, or a missing key) -- reported, never renormalized away, so
///   `sum(frequency) + unresolved_mass = 1` whenever frequencies exist.
/// - `range_mix`: the mass-weighted action frequencies over hero's public range at the node,
///   over the node-covered branches only (the excluded share is disclosed in `notes`); present
///   whenever a node strategy exists, and the only strategy output under `HeroComboOutOfSupport`.
/// - `reasons`: `ChartRounded` / `EvReferenceUnverified` for the sources that contributed, and,
///   whenever `unresolved_mass > 0`, one `BranchResidual` per cause actually incurred (spec section
///   8.4's vocabulary; ruling 19-I1): `cause: "cap"` with the cap residual's posterior share, and
///   `cause: "missing node <key>"` with the share of the positive branches whose key has no node;
///   their shares sum to `unresolved_mass`. No duplicates.
/// - `notes`: `"x% of the posterior has no strategy"` whenever `unresolved_mass > 0`, and the
///   range mix's excluded share.
/// - `unsupported`: `MissingPreflopNode { key }` when hero has a node in no positive-posterior
///   branch; `HeroComboOutOfSupport` when hero's combo has zero public mass or a node marks its
///   class explicitly unreachable.
#[derive(Clone, Debug, PartialEq)]
pub struct MixedNode {
    pub actions: Vec<ActionAdvice>,
    pub unresolved_mass: f32,
    pub range_mix: Option<Vec<(Action, f32)>>,
    pub reasons: Vec<ApproxReason>,
    pub notes: Vec<String>,
    pub unsupported: Option<UnsupportedReason>,
}

/// Section 8.4's per-action assembly kernel for hero's combo `c`, over the history branches `k`.
///
/// `posterior[k]` is hero's branch posterior `pi_{H,k}[c]` (a distribution: it sums to 1, or is
/// all zeros when the combo has no public mass); `probs[k]` is `P_k(a | c)` when branch `k`'s node
/// lists this action (identical kind and identical mapped chip amount), `None` when the branch has
/// no node or its menu lacks the action; `evs[k]` is branch `k`'s normalized chip EV for the
/// action, `None` when it has none; `same_reference` says every contributing node carries the same
/// `EvReference` (compared before any averaging).
///
/// Returns `(frequency, ev, unavailable)`:
/// - `frequency = sum_k pi_k * P_k(a | c)`, the known mass of the action: a branch without the
///   action (or without a node) contributes nothing, and nothing is renormalized.
/// - With `B+ = {k : pi_k > 0}`: `ev = sum_{k in B+} pi_k * EV_k` -- a plain posterior average,
///   never weighted by action frequency and never divided by the covered posterior -- defined only
///   when every branch of `B+` has the action with an EV and `same_reference` holds.
/// - Otherwise `ev = None` and, in this order: `NotInMenu` when no branch of `B+` has the action;
///   `BranchSupportIncomplete { covered_posterior }` (the `B+` posterior whose branch has the
///   action with an EV, possibly 0) when some branch of `B+` lacks the action or a node;
///   `NoEvReference` when every branch has the action but some EV is missing or the references
///   differ. [`mix_nodes`] refines `NoEvReference` into `ChartNoEv` or `MovedProbability` where
///   that is the actual cause.
///
/// # Panics
/// Always (not only in debug builds), naming the offending index, if the three slices differ in
/// length, a posterior entry is not a finite value in `[0, 1]`, the posterior neither sums to 1
/// (within [`MASS_TOLERANCE`]) nor is all zero, a probability is not a finite value in `[0, 1]`,
/// an EV is not finite, or an EV is given for an action absent from its branch.
pub fn mix_action(posterior: &[f64], probs: &[Option<f64>], evs: &[Option<f64>], same_reference: bool) -> (f64, Option<f64>, Option<Unavailable>) {
    check_mix_inputs(posterior, probs, evs);
    let pi = posterior;
    let freq: f64 = pi.iter().zip(probs).map(|(w, p)| w * p.unwrap_or(0.0)).sum();
    let active: Vec<usize> = (0..pi.len()).filter(|&k| pi[k] > 0.0).collect();
    let present = active.iter().filter(|&&k| probs[k].is_some()).count();
    if present == 0 {
        return (freq, None, Some(Unavailable::NotInMenu));
    }
    let with_ev = |k: usize| probs[k].is_some() && evs[k].is_some();
    let covered: f64 = active.iter().filter(|&&k| with_ev(k)).map(|&k| pi[k]).sum();
    let complete = active.iter().all(|&k| with_ev(k));
    if complete && same_reference {
        let ev: f64 = active.iter().map(|&k| pi[k] * evs[k].expect("complete support: every active branch has an EV")).sum();
        return (freq, Some(ev), None);
    }
    if present != active.len() {
        let covered_posterior = narrow_share(covered, "mix_action: covered posterior");
        return (freq, None, Some(Unavailable::BranchSupportIncomplete { covered_posterior }));
    }
    (freq, None, Some(Unavailable::NoEvReference))
}

/// [`mix_action`]'s always-on input checks: shape first, then the value domains.
fn check_mix_inputs(pi: &[f64], probs: &[Option<f64>], evs: &[Option<f64>]) {
    assert!(probs.len() == pi.len(), "mix_action: posterior has {} entries but probs has {}", pi.len(), probs.len());
    assert!(evs.len() == pi.len(), "mix_action: posterior has {} entries but evs has {}", pi.len(), evs.len());
    for (k, w) in pi.iter().enumerate() {
        assert!(w.is_finite() && (0.0..=1.0).contains(w), "mix_action: posterior[{k}] = {w} is not a probability in [0, 1]");
    }
    let total: f64 = pi.iter().sum();
    assert!(total == 0.0 || (total - 1.0).abs() <= MASS_TOLERANCE, "mix_action: posterior sums to {total}, not 1");
    for (k, p) in probs.iter().enumerate() {
        if let Some(p) = p {
            assert!(p.is_finite() && (0.0..=1.0).contains(p), "mix_action: probs[{k}] = {p} is not a probability in [0, 1]");
        }
    }
    for (k, ev) in evs.iter().enumerate() {
        if let Some(v) = ev {
            assert!(v.is_finite(), "mix_action: evs[{k}] = {v} is not finite");
            assert!(probs[k].is_some(), "mix_action: evs[{k}] is present for an action absent from branch {k}");
        }
    }
}

/// Hero's current preflop decision over the shared history branches (spec section 8.4's node
/// translation and assembly, section 6's coverage rows, section 4.4's `unresolved_mass` and
/// `range_mix`). `nodes` holds at most one [`BranchNode`] per branch (a branch with no entry has
/// no node); `hero_combo` only selects hero's posterior -- hero's cards never enter any public
/// quantity -- and `bb_chips` converts the final chip EV to bb once.
///
/// With `pi = posterior(branches, hero, hero_combo)` and `B+ = {k : pi_k > 0}`:
///
/// 0. **Source-rounding policy** (fix round 1, T16-R1), one rule at the mapped-node boundary:
///    every supplied node's probability rows are admitted and normalized once, in `f64`, before
///    any mixing. A reachable row whose sum `s` lies within spec 8.2's admission tolerance
///    `1 +- 1e-3` (plus [`F32_CARRIER_SLACK`] for its `f32` carrier) is divided by `s`, so its
///    probabilities sum to 1 (to `f64` rounding) and each lies in `[0, 1]` -- which is what the
///    assembly's `[0, 1]` checks and [`mix_action`] then see; a reachable row outside it is
///    rejected, naming the node and the sum; an explicitly unreachable row must be all zero and is
///    kept as is. Only source rows are normalized: the branch posterior, `unresolved_mass` and the
///    kernel's [`MASS_TOLERANCE`] are untouched, and nothing is renormalized over the covered
///    branches.
/// 1. A branch **has a node** only when it is neither the residual nor stopped (spec section 9.3:
///    a stopped branch never resumes at a later present node, and for lookups behaves like the
///    residual) and its entry carries a node. `unresolved_mass = sum_{k without node} pi_k`.
/// 2. No node in any branch of `B+` (or in any branch at all): `Unsupported { MissingPreflopNode {
///    key } }`, with no advice and no range mix. `key` is chosen from the heaviest (`q`, ties by
///    creation order) positive-posterior branch, selected **before** the residual is excluded
///    (fix round 1, T16-R2): its retained key; if it has no source key -- the residual never has
///    one -- the heaviest known stopped/live key across **all** branches; never an empty invented
///    node.
/// 3. Hero's combo has zero public mass (`B+` empty), or a node of some branch in `B+` marks its
///    class explicitly unreachable: `Unsupported { HeroComboOutOfSupport }`; every union action
///    is listed with no frequency and no EV (`HeroOutOfSupport`), and the range-level mix is the
///    only strategy output. It is never replaced by a nearby hand's strategy.
/// 4. Otherwise every action of the union of the mapped menus goes through [`mix_action`] (an
///    action is present in a branch only under identical kind and identical chip amount), with
///    `same_reference` over the nodes of `B+`. A `NoEvReference` (complete action support, EV
///    missing) is refined: `ChartNoEv` when every contributing node is a chart; `MovedProbability
///    { from }` when every contributing node is a verified PokerData node under one reference and
///    every missing EV belongs to a legality-created destination ([`BranchNode::created`]; `from`
///    of the first such branch); otherwise it stays `NoEvReference`. `BranchSupportIncomplete`
///    and `NotInMenu` are never refined.
///
/// `range_mix` weights each combo by `sum_{k has node} q_k * w_{H,k}[c]`, independently of hero's
/// combo, and divides by that covered mass (the excluded share is disclosed in `notes`).
/// Whenever `unresolved_mass > 0`, hero's unresolved share is disclosed by cause (spec section 8.4's
/// `"cap" | "missing node <key>"`; section 2: only reasons actually incurred; ruling 19-I1, which
/// replaces plan 3 Task 16's single `"missing node"` wording): the persistent cap residual's
/// posterior share as `BranchResidual { seat: hero, residual_mass_pct: 100 * pi_R, cause: "cap" }`,
/// then the share of the positive branches whose key has no node (stopped, or live without a node)
/// as `BranchResidual { seat: hero, residual_mass_pct: 100 * sum pi_k, cause: "missing node <key>" }`
/// naming the heaviest such branch's retained key (`"no retained key"` when none retains one --
/// never a key whose node is present). A residual-only share therefore has only the cap reason, and
/// both shares sum to `unresolved_mass`, whose total is the note `"x% of the posterior has no
/// strategy"`. Contributing chart nodes add `ChartRounded`
/// and unverified PokerData nodes add `EvReferenceUnverified`. Branches are read, never mutated:
/// stopping a branch belongs to the replay walk, and nothing is renormalized.
///
/// # Panics
/// Always (not only in debug builds), naming the offending branch, action or combo, if `bb_chips`
/// is 0, `hero_combo` is not a combo index, two branches share an id, a node entry names no branch
/// or a branch twice, a node is not hero's, lists an action twice, has other than 1326 rows of its
/// menu's length, holds a negative or non-finite probability, a reachable row whose sum is outside
/// the admission tolerance of step 0, an explicitly unreachable row that is not all zero, or a
/// non-finite EV, a created destination is off its menu, names itself or carries an EV, or through
/// [`posterior`]'s own checks.
pub fn mix_nodes(branches: &[HistoryBranch], nodes: &[BranchNode], hero: Seat, hero_combo: usize, bb_chips: u32) -> MixedNode {
    assert!(bb_chips > 0, "mix_nodes: bb_chips is 0");
    assert!(hero_combo < COMBOS, "mix_nodes: hero combo {hero_combo} is out of range 0..{COMBOS}");
    let entry = branch_entries(branches, nodes, hero);
    let pi = posterior(branches, hero, hero_combo);
    let all: Vec<usize> = (0..branches.len()).collect();
    let unresolved: f64 = all.iter().filter(|&&k| entry[k].is_none()).map(|&k| pi[k]).sum();
    let positive: Vec<usize> = all.iter().copied().filter(|&k| pi[k] > 0.0).collect();
    if !entry.iter().any(Option::is_some) || (!positive.is_empty() && positive.iter().all(|&k| entry[k].is_none())) {
        let candidates = if positive.is_empty() { &all } else { &positive };
        return MixedNode {
            actions: vec![],
            unresolved_mass: narrow_share(unresolved, "mix_nodes: unresolved mass"),
            range_mix: None,
            reasons: vec![],
            notes: vec![],
            unsupported: Some(UnsupportedReason::MissingPreflopNode { key: missing_node_key(branches, nodes, candidates) }),
        };
    }
    let menu = union_menu(&entry);
    let covered_nodes: Vec<&ExpandedNode> = entry.iter().flatten().map(|u| u.node).collect();
    let (range_mix, excluded) = range_mix(branches, &entry, hero, &menu);
    let mut out = MixedNode {
        actions: vec![],
        unresolved_mass: narrow_share(unresolved, "mix_nodes: unresolved mass"),
        range_mix,
        reasons: source_reasons(&covered_nodes),
        notes: vec![],
        unsupported: None,
    };
    if excluded > 0.0 {
        out.notes.push(format!("range mix excludes {:.1}% of hero's public range mass (no strategy)", 100.0 * excluded));
    }
    let out_of_support = positive.is_empty() || positive.iter().any(|&k| entry[k].as_ref().is_some_and(|u| !u.node.available[hero_combo]));
    if out_of_support {
        out.actions = menu
            .iter()
            .map(|&action| ActionAdvice { action, frequency: None, ev_bb: None, unavailable: Some(Unavailable::HeroOutOfSupport), headline: false })
            .collect();
        out.unsupported = Some(UnsupportedReason::HeroComboOutOfSupport);
    } else {
        let contributing: Vec<&Usable> = positive.iter().filter_map(|&k| entry[k].as_ref()).collect();
        let same = contributing.windows(2).all(|w| w[0].node.ev_reference == w[1].node.ev_reference);
        for action in menu {
            let probs: Vec<Option<f64>> = entry.iter().map(|e| e.as_ref().and_then(|u| node_prob(u, &action, hero_combo))).collect();
            let evs: Vec<Option<f64>> = entry.iter().map(|e| e.as_ref().and_then(|u| node_ev(u, &action, hero_combo))).collect();
            let (freq, ev, why) = mix_action(&pi, &probs, &evs, same);
            let unavailable = refine_no_ev(why, &action, &contributing, hero_combo, same);
            out.actions.push(ActionAdvice {
                action,
                frequency: Some(narrow_share(freq, "mix_nodes: frequency")),
                ev_bb: ev.map(|v| to_bb(v, bb_chips)),
                unavailable,
                headline: false,
            });
        }
    }
    if unresolved > 0.0 {
        let pct = 100.0 * unresolved;
        assert!(pct.is_finite() && pct <= 100.0 * (1.0 + MASS_TOLERANCE), "mix_nodes: unresolved share {pct}% is not a percentage");
        // Ruling 19-I1 (spec sections 2 and 8.4): one reason per cause actually incurred -- the cap
        // residual's share first, then the share of the positive branches whose key has no node.
        let (capped, missing): (Vec<usize>, Vec<usize>) =
            positive.iter().copied().filter(|&k| entry[k].is_none()).partition(|&k| branches[k].residual);
        if !capped.is_empty() {
            push_unique(&mut out.reasons, residual_share(hero, &pi, &capped, "cap".into()));
        }
        if !missing.is_empty() {
            let cause = format!("missing node {}", residual_cause_key(branches, nodes, &missing));
            push_unique(&mut out.reasons, residual_share(hero, &pi, &missing, cause));
        }
        out.notes.push(format!("{pct:.1}% of the posterior has no strategy"));
    }
    out
}

/// Hero's `BranchResidual` for one cause: the posterior share of branches `ks`, as a percentage.
///
/// # Panics
/// Always, if that share is not a percentage (at most `100 * (1 + MASS_TOLERANCE)`).
fn residual_share(hero: Seat, pi: &[f64], ks: &[usize], cause: String) -> ApproxReason {
    let pct = 100.0 * ks.iter().map(|&k| pi[k]).sum::<f64>();
    assert!(pct.is_finite() && pct <= 100.0 * (1.0 + MASS_TOLERANCE), "mix_nodes: residual share {pct}% ({cause}) is not a percentage");
    ApproxReason::BranchResidual {
        seat: hero,
        // Positive stays positive at the f32 boundary; `pct <= 100 * (1 + MASS_TOLERANCE)` narrows
        // to at most exactly 100.
        residual_mass_pct: pct.max(f64::from(f32::MIN_POSITIVE)) as f32,
        cause,
    }
}

/// The covered-mass weight of combo `c` in hero's range mix: `sum_{k in covered} q_k *
/// w_{H,k}[c]` over the branches whose ids are in `covered` (spec section 8.4: "the range mix uses
/// `sum_{k has node} q_k * w_{H,k}[c]` as the mass over the branches with nodes"). Independent of
/// hero's actual combo.
///
/// # Panics
/// Always, if `c` is not a combo index or a covered branch lacks hero's seat.
pub fn range_mix_weight(branches: &[HistoryBranch], hero: Seat, c: usize, covered: &[u8]) -> f64 {
    assert!(c < COMBOS, "range_mix_weight: combo {c} is out of range 0..{COMBOS}");
    branches
        .iter()
        .filter(|b| covered.contains(&b.id))
        .map(|b| b.q * b.seats.iter().find(|s| s.seat == hero).unwrap_or_else(|| panic!("range_mix_weight: branch {} has no seat {hero:?}", b.id)).mass[c])
        .sum()
}

/// One branch's usable node for the assembly: its entry, the entry's node, and the node's
/// probability rows admitted and normalized once at the mapped-node boundary ([`admit_row`]).
/// Every probability the assembly mixes -- per combo and in the range mix -- is read from `probs`,
/// never from the node's raw `f32` rows.
struct Usable<'a> {
    entry: &'a BranchNode,
    node: &'a ExpandedNode,
    probs: Vec<Vec<f64>>,
}

/// One entry per branch: the branch's [`Usable`] node when the branch **has a node** (it is neither
/// the residual nor stopped, and its entry carries a node), else `None`. Validates and admits every
/// entry -- including a node supplied for a frozen branch, which is ignored rather than resumed.
fn branch_entries<'a>(branches: &[HistoryBranch], nodes: &'a [BranchNode], hero: Seat) -> Vec<Option<Usable<'a>>> {
    for (k, b) in branches.iter().enumerate() {
        assert!(branches[..k].iter().all(|x| x.id != b.id), "mix_nodes: branch id {} appears more than once", b.id);
    }
    let mut entry: Vec<Option<Usable>> = (0..branches.len()).map(|_| None).collect();
    let mut seen = vec![false; branches.len()];
    for n in nodes {
        let k = branches
            .iter()
            .position(|b| b.id == n.branch_id)
            .unwrap_or_else(|| panic!("mix_nodes: node for branch {} names no branch in the list", n.branch_id));
        assert!(!seen[k], "mix_nodes: branch {} has more than one node entry", n.branch_id);
        seen[k] = true;
        match &n.node {
            Some(node) => {
                let probs = check_node(n.branch_id, &n.key, node, &n.created, hero);
                if !branches[k].residual && branches[k].stopped.is_none() {
                    entry[k] = Some(Usable { entry: n, node, probs });
                }
            }
            None => assert!(n.created.is_empty(), "mix_nodes: branch {} lists created destinations but has no node", n.branch_id),
        }
    }
    entry
}

/// [`mix_nodes`]'s always-on checks of one mapped node and its created destinations. Returns the
/// node's probability rows admitted and normalized by [`admit_row`] (step 0 of [`mix_nodes`]).
fn check_node(id: u8, key: &str, n: &ExpandedNode, created: &[(Action, Action)], hero: Seat) -> Vec<Vec<f64>> {
    assert!(n.actor == hero, "mix_nodes: branch {id}'s node is seat {:?}'s, not hero {hero:?}'s", n.actor);
    for (i, a) in n.actions.iter().enumerate() {
        if let Some(j) = (i + 1..n.actions.len()).find(|&j| n.actions[j] == *a) {
            panic!("mix_nodes: branch {id}'s node lists {a:?} twice (actions {i} and {j})");
        }
    }
    for (what, len) in [("probability", n.probs.len()), ("EV", n.ev_chips.len()), ("availability", n.available.len())] {
        assert!(len == COMBOS, "mix_nodes: branch {id}'s node has {len} {what} rows, expected {COMBOS}");
    }
    let mut rows = Vec::with_capacity(COMBOS);
    for c in 0..COMBOS {
        let (probs, evs) = (&n.probs[c], &n.ev_chips[c]);
        assert!(
            probs.len() == n.actions.len() && evs.len() == n.actions.len(),
            "mix_nodes: branch {id}'s node combo {c} has {} probabilities and {} EVs for {} actions",
            probs.len(),
            evs.len(),
            n.actions.len()
        );
        for (i, p) in probs.iter().enumerate() {
            assert!(p.is_finite() && *p >= 0.0, "mix_nodes: branch {id}'s node combo {c} action {i} probability {p} is not a finite non-negative value");
        }
        for (i, ev) in evs.iter().enumerate() {
            if let Some(v) = ev {
                assert!(v.is_finite(), "mix_nodes: branch {id}'s node combo {c} action {i} EV {v} is not finite");
            }
        }
        rows.push(admit_row(id, key, c, probs, n.available[c]));
    }
    for (j, (dest, from)) in created.iter().enumerate() {
        let i = n
            .actions
            .iter()
            .position(|a| a == dest)
            .unwrap_or_else(|| panic!("mix_nodes: branch {id}'s created destination {dest:?} is not on its menu"));
        assert!(dest != from, "mix_nodes: branch {id}'s created destination {dest:?} names itself as its source");
        assert!(created[..j].iter().all(|(d, _)| d != dest), "mix_nodes: branch {id} lists created destination {dest:?} twice");
        assert!(
            n.ev_chips.iter().all(|row| row[i].is_none()),
            "mix_nodes: branch {id}'s created destination {dest:?} carries an EV (a created destination owns none)"
        );
    }
    rows
}

/// The mapped-node boundary's single source-rounding policy (fix round 1, T16-R1; step 0 of
/// [`mix_nodes`]) for one combo row of finite, non-negative probabilities with `f64` sum `s`:
///
/// - explicitly unreachable (`available` false): spec 8.2 requires the row to sum to exactly 0,
///   i.e. to be all zero, and it is kept as is;
/// - reachable: admitted when `|s - 1| <= SOURCE_ROW_TOLERANCE + F32_CARRIER_SLACK` and divided by
///   `s` once, so the row sums to 1 (to `f64` rounding) and every entry lies in `[0, 1]` (a
///   correctly rounded `p / s` never exceeds 1 for `p <= s`); rejected otherwise.
///
/// This normalizes source rounding only -- a row the source itself admitted -- never a branch
/// posterior or an unresolved mass, and it never clamps: a row outside the tolerance is an error.
///
/// # Panics
/// Always, naming the node (branch and key), the combo and the sum, if the row is outside the
/// tolerance or an unreachable row is not all zero.
fn admit_row(id: u8, key: &str, c: usize, probs: &[f32], available: bool) -> Vec<f64> {
    let wide: Vec<f64> = probs.iter().map(|&p| f64::from(p)).collect();
    let sum: f64 = wide.iter().sum();
    let row = if available {
        assert!(
            (sum - 1.0).abs() <= SOURCE_ROW_TOLERANCE + F32_CARRIER_SLACK,
            "mix_nodes: branch {id}'s node {key:?} combo {c} probabilities sum to {sum}, outside spec 8.2's 1 +- {SOURCE_ROW_TOLERANCE:e} (f32 carrier slack {F32_CARRIER_SLACK:e})"
        );
        wide.iter().map(|p| p / sum).collect()
    } else {
        assert!(sum == 0.0, "mix_nodes: branch {id}'s node {key:?} combo {c} is explicitly unreachable but its probabilities sum to {sum}, not exactly 0");
        wide
    };
    for (i, p) in row.iter().enumerate() {
        assert!((0.0..=1.0).contains(p), "mix_nodes: branch {id}'s node combo {c} action {i} normalized probability {p} is not in [0, 1]");
    }
    row
}

/// The union of the mapped menus in the canonical order ([`menu_rank`]); actions are identified by
/// kind and chip amount, so a raise to 12 and a raise to 13 stay two actions.
fn union_menu(entry: &[Option<Usable>]) -> Vec<Action> {
    let mut menu: Vec<Action> = Vec::new();
    for u in entry.iter().flatten() {
        for a in &u.node.actions {
            if !menu.contains(a) {
                menu.push(*a);
            }
        }
    }
    menu.sort_by_key(menu_rank);
    menu
}

/// `P_k(a | c)`, from the admitted and normalized row, when the branch's node lists `action`
/// (identical kind and chip amount).
fn node_prob(u: &Usable, action: &Action, c: usize) -> Option<f64> {
    u.node.actions.iter().position(|a| a == action).map(|i| u.probs[c][i])
}

/// Branch `k`'s normalized chip EV for `action` and combo `c`, when its node lists the action and
/// carries one.
fn node_ev(u: &Usable, action: &Action, c: usize) -> Option<f64> {
    u.node.actions.iter().position(|a| a == action).and_then(|i| u.node.ev_chips[c][i]).map(f64::from)
}

/// The source action whose probability created `action` on this branch's menu, if it was created.
fn created_from(u: &Usable, action: &Action) -> Option<Action> {
    u.entry.created.iter().find(|(d, _)| d == action).map(|(_, from)| *from)
}

/// Refines [`mix_action`]'s `NoEvReference` (complete action support, some EV missing or the
/// references differ) into the actual cause: `ChartNoEv` when every contributing node is a chart;
/// `MovedProbability { from }` when every contributing node is a verified PokerData node under one
/// reference and each missing EV belongs to a legality-created destination; else unchanged.
fn refine_no_ev(why: Option<Unavailable>, action: &Action, contributing: &[&Usable], c: usize, same: bool) -> Option<Unavailable> {
    if why != Some(Unavailable::NoEvReference) {
        return why;
    }
    if contributing.iter().all(|u| u.node.source == SourceKind::ChartTranscription) {
        return Some(Unavailable::ChartNoEv);
    }
    let verified = contributing.iter().all(|u| u.node.source == SourceKind::PokerDataJson && u.node.ev_reference != EvReference::Unverified);
    if same && verified {
        let missing: Vec<&&Usable> = contributing.iter().filter(|u| node_ev(u, action, c).is_none()).collect();
        if let Some(first) = missing.first() {
            if missing.iter().all(|u| created_from(u, action).is_some()) {
                return Some(Unavailable::MovedProbability { from: created_from(first, action).expect("checked just above") });
            }
        }
    }
    why
}

/// The range-level mix over the node-covered branches (spec section 8.4) and the share of hero's
/// public range mass it excludes (branches with no node). `None` when the covered mass is zero.
fn range_mix(branches: &[HistoryBranch], entry: &[Option<Usable>], hero: Seat, menu: &[Action]) -> (Option<Vec<(Action, f32)>>, f64) {
    let covered_ids: Vec<u8> = branches.iter().zip(entry).filter(|(_, e)| e.is_some()).map(|(b, _)| b.id).collect();
    let uncovered_ids: Vec<u8> = branches.iter().zip(entry).filter(|(_, e)| e.is_none()).map(|(b, _)| b.id).collect();
    let covered: f64 = (0..COMBOS).map(|c| range_mix_weight(branches, hero, c, &covered_ids)).sum();
    let uncovered: f64 = (0..COMBOS).map(|c| range_mix_weight(branches, hero, c, &uncovered_ids)).sum();
    let total = covered + uncovered;
    let excluded = if total > 0.0 { uncovered / total } else { 0.0 };
    if !(covered > 0.0) {
        return (None, excluded);
    }
    let mut numerators = vec![0.0_f64; menu.len()];
    for (b, e) in branches.iter().zip(entry) {
        let Some(u) = e else { continue };
        let w = &b.seats.iter().find(|s| s.seat == hero).expect("range_mix_weight checked hero's seat").mass;
        for (j, a) in menu.iter().enumerate() {
            if let Some(i) = u.node.actions.iter().position(|x| x == a) {
                numerators[j] += (0..COMBOS).map(|c| b.q * w[c] * u.probs[c][i]).sum::<f64>();
            }
        }
    }
    let mix = menu.iter().zip(numerators).map(|(a, x)| (*a, narrow_share(x / covered, "mix_nodes: range mix"))).collect();
    (Some(mix), excluded)
}

/// `ChartRounded` for any contributing chart node and `EvReferenceUnverified` for any contributing
/// PokerData node with an unverified reference (spec sections 6 and 8.3), each at most once.
fn source_reasons(nodes: &[&ExpandedNode]) -> Vec<ApproxReason> {
    let mut reasons = Vec::new();
    if nodes.iter().any(|n| n.source == SourceKind::ChartTranscription) {
        push_unique(&mut reasons, ApproxReason::ChartRounded);
    }
    if nodes.iter().any(|n| n.source == SourceKind::PokerDataJson && n.ev_reference == EvReference::Unverified) {
        push_unique(&mut reasons, ApproxReason::EvReferenceUnverified);
    }
    reasons
}

/// Reasons accumulate without duplicates.
fn push_unique(reasons: &mut Vec<ApproxReason>, reason: ApproxReason) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

/// The label used when no candidate branch retains any key -- never an empty key, and never another
/// branch's key.
const NO_RETAINED_KEY: &str = "no retained key";

/// Branch `k`'s retained key: the lookup key of its [`BranchNode`] entry, when it has an entry
/// with a non-empty key (the residual has no source key).
fn key_of<'a>(branches: &[HistoryBranch], nodes: &'a [BranchNode], k: usize) -> Option<&'a str> {
    nodes.iter().find(|n| n.branch_id == branches[k].id).map(|n| n.key.as_str()).filter(|s| !s.is_empty())
}

/// The heaviest branch among `ks`: the largest `q`, ties to the earlier-created branch (lower id).
fn heaviest(branches: &[HistoryBranch], ks: impl Iterator<Item = usize>) -> Option<usize> {
    ks.max_by(|&a, &b| branches[a].q.total_cmp(&branches[b].q).then(branches[b].id.cmp(&branches[a].id)))
}

/// The heaviest known stopped/live key among `ks`: the key of the heaviest non-residual branch of
/// `ks` that retains one.
fn heaviest_known_key(branches: &[HistoryBranch], nodes: &[BranchNode], ks: impl Iterator<Item = usize>) -> Option<String> {
    let keyed = ks.filter(|&k| !branches[k].residual && key_of(branches, nodes, k).is_some());
    heaviest(branches, keyed).and_then(|k| key_of(branches, nodes, k)).map(str::to_string)
}

/// `MissingPreflopNode`'s key when hero has a node in no positive-posterior branch (spec section
/// 8.4: "the key of the heaviest branch (largest `q_k`)"; fix round 1, T16-R2): the heaviest branch
/// of `candidates` is selected **before** the residual is excluded (ties by creation order); its
/// retained key when it has one; if it has no source key -- the residual never has one -- the
/// heaviest known stopped/live key across **all** branches, zero-posterior ones included; else
/// [`NO_RETAINED_KEY`].
fn missing_node_key(branches: &[HistoryBranch], nodes: &[BranchNode], candidates: &[usize]) -> String {
    heaviest(branches, candidates.iter().copied())
        .filter(|&k| !branches[k].residual)
        .and_then(|k| key_of(branches, nodes, k))
        .map(str::to_string)
        .or_else(|| heaviest_known_key(branches, nodes, 0..branches.len()))
        .unwrap_or_else(|| NO_RETAINED_KEY.into())
}

/// The key in the partial-coverage `BranchResidual` cause ("missing node <key>"): the heaviest
/// known key among `missing`, the positive-posterior branches (never the residual) whose key has no
/// node; [`NO_RETAINED_KEY`] when none of them retains one. Ruling 19-I1: never another branch's
/// key, whose node is present -- the residual's own share is disclosed as the cap instead.
fn residual_cause_key(branches: &[HistoryBranch], nodes: &[BranchNode], missing: &[usize]) -> String {
    heaviest_known_key(branches, nodes, missing.iter().copied()).unwrap_or_else(|| NO_RETAINED_KEY.into())
}

/// Narrows a probability share (a frequency, a posterior mass, a range-mix weight) from `f64` to
/// the `f32` wire type -- the assembly's only narrowing of shares, after validation. Zero stays
/// zero; a positive share below `f32::MIN_POSITIVE` becomes `f32::MIN_POSITIVE`, so a positive
/// unresolved mass can never narrow to the zero that would allow a headline (spec section 9.2's
/// output-boundary rule).
///
/// # Panics
/// Always, naming `what`, if `x` is not a finite value in `[0, 1 + MASS_TOLERANCE]`: out-of-domain
/// input is an error, never clamped.
fn narrow_share(x: f64, what: &str) -> f32 {
    assert!(x.is_finite() && (0.0..=1.0 + MASS_TOLERANCE).contains(&x), "{what} = {x} is not a share in [0, 1]");
    // `x <= 1 + MASS_TOLERANCE` lies within half an `f32` ulp of 1, so the narrowed share is at
    // most exactly `1.0_f32`: nothing is clamped from above.
    if x == 0.0 {
        0.0
    } else {
        x.max(f64::from(f32::MIN_POSITIVE)) as f32
    }
}

/// A mixed chip EV in bb (spec section 8.3: `ev_chips / bb_chips`), converted once, in `f64`, and
/// narrowed to `f32` at the boundary.
///
/// # Panics
/// Always, if the narrowed value is not finite.
fn to_bb(ev_chips: f64, bb_chips: u32) -> f32 {
    let out = (ev_chips / f64::from(bb_chips)) as f32;
    assert!(out.is_finite(), "mix_nodes: EV {ev_chips} chips at {bb_chips} chips per bb is not a finite f32");
    out
}
