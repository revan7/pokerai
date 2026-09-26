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

use proto::{Action, LegalAction, Unavailable, UnsupportedReason};

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

/// Sorts `menu` into the canonical order (`Fold`, `Check`, `Call`, wagers ascending by `to`,
/// `AllIn`) and rewrites `created`/`mapped` to the same permutation, so every row of a node sees
/// an identical menu order regardless of the order source actions happened to arrive in.
fn order_menu(menu: &mut Vec<Action>, created: &mut Vec<bool>, mapped: &mut [Option<usize>]) {
    fn rank(a: &Action) -> (u8, u32) {
        match a {
            Action::Fold => (0, 0),
            Action::Check => (1, 0),
            Action::Call => (2, 0),
            Action::Bet { to } | Action::Raise { to } => (3, *to),
            Action::AllIn { to } => (4, *to),
        }
    }
    let mut order: Vec<usize> = (0..menu.len()).collect();
    order.sort_by_key(|&i| rank(&menu[i]));
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
/// reading the output as a distribution.
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
