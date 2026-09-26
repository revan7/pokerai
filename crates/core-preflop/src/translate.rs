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
    let fa = (b - s) * (1.0 + a) / ((b - a) * (1.0 + s));
    Some(Interpolation { choices: vec![(i, fa), (j, 1.0 - fa)], deviation: (s - a).abs().min((s - b).abs()), clamped: false })
}

/// The observed wager's pot fraction at the parent node (spec section 8.4): `s = (chips added
/// beyond a call) / (pot after the bettor's call)`. `own` is the actor's own contribution this
/// street before this wager; `call` is what the actor owed to match the current wager (`0` for an
/// opening bet); `pot` is the pot **before** this actor's call is added, reconstructed from the
/// financial state at the mapped parent node -- never a later observed pot. For a raise-to `to`,
/// this is `(to - own - call) / (pot + call)`.
///
/// # Panics
/// Panics (standing ruling: an infallible-signature function enforces its invariants with an
/// always-on `assert!`, naming the offending inputs) if `pot + call` is zero -- a zero-pot input
/// has no meaningful pot fraction, and this function's signature (fixed by the task brief) returns
/// a bare `f64`, not an `Option`/`Result`, so the only way to reject that domain is to panic
/// rather than return a silently meaningless (`NaN`/infinite) fraction -- or if `to` is less than
/// `own + call`, which is not a wager at all (a "negative wager" the standing ruling calls out as
/// invalid input).
pub fn wager_fraction(to: u32, own: u32, call: u32, pot: u32) -> f64 {
    let denom = pot as f64 + call as f64;
    let numer = to as f64 - own as f64 - call as f64;
    assert!(denom > 0.0, "wager_fraction: pot {pot} + call {call} must be positive (a zero pot is invalid input)");
    assert!(numer >= 0.0, "wager_fraction: to {to} must be at least own {own} + call {call} (negative wagers are invalid input)");
    numer / denom
}

/// A source node's own wager-sized actions (`Bet`/`Raise`/`AllIn`), converted to the
/// `(menu index, pot fraction)` pairs [`interpolate`]'s `menu` parameter expects, at one parent
/// node's financial context (`own`/`call`/`pot`, the same meaning as [`wager_fraction`]'s). `Fold`,
/// `Check` and `Call` carry no size and are skipped; the returned index is that action's own
/// position within `actions`, so a caller can look the chosen menu action back up directly.
pub fn menu_fractions(actions: &[Action], own: u32, call: u32, pot: u32) -> Vec<(usize, f64)> {
    actions
        .iter()
        .enumerate()
        .filter_map(|(i, a)| match a {
            Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => Some((i, wager_fraction(*to, own, call, pot))),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Legality after mapping (spec section 8.4): one destination map per node, walked once per row.
// ---------------------------------------------------------------------------------------------

/// Where one source action's probability/EV mass lands in the legalized output menu, and whether
/// that destination exists on the menu only because this move put it there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Destination {
    pub index: usize,
    pub created: bool,
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
/// AllIn) plus, for each source action, where its probability/EV mass lands and whether that
/// destination exists on the menu only because this move put it there. Built once per node and
/// reused for every one of the 1326 combo rows via [`legalize_row`] -- a single per-combo
/// `legalize` could not express this shared map, which is why the two halves are separate
/// functions.
///
/// Pass 1 gives every source action that is itself legal its own destination. Pass 2 moves every
/// remaining (illegal) source action by kind: a below-minimum `Raise` goes to the smallest legal
/// source-menu raise, or to `Call` if none is legal; a `Bet` or `Raise` above the legal maximum
/// goes to the legal `AllIn`; anything else (including a below-minimum `Bet`, which section 8.4
/// gives no fallback rule for) has no legal destination at all and is rejected to the caller as
/// `UnsupportedReason::UnsupportedHistory` -- never guessed as a fold or silently dropped.
pub fn destination_map(actions: &[Action], legal: &[LegalAction]) -> Result<(Vec<Action>, Vec<Destination>), UnsupportedReason> {
    let mut menu: Vec<Action> = Vec::new();
    let mut created: Vec<bool> = Vec::new();
    let push = |menu: &mut Vec<Action>, created: &mut Vec<bool>, a: Action, new: bool| -> usize {
        match menu.iter().position(|x| *x == a) {
            Some(i) => {
                if !new {
                    created[i] = false;
                }
                i
            }
            None => {
                menu.push(a);
                created.push(new);
                menu.len() - 1
            }
        }
    };
    // Pass 1: every source action that is itself legal owns its destination.
    let mut mapped: Vec<Option<usize>> = vec![None; actions.len()];
    for (k, a) in actions.iter().enumerate() {
        if is_legal(a, legal) {
            mapped[k] = Some(push(&mut menu, &mut created, *a, false));
        }
    }
    // Pass 2: illegal source actions move by kind (spec section 8.4's legality-after-mapping rule).
    for (k, a) in actions.iter().enumerate() {
        if mapped[k].is_some() {
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
        // `new` is true only when this move is what puts the destination on the menu.
        let new = !menu.contains(&t) && !actions.iter().any(|x| *x == t && is_legal(x, legal));
        mapped[k] = Some(push(&mut menu, &mut created, t, new));
    }
    order_menu(&mut menu, &mut created, &mut mapped);
    Ok((
        menu,
        mapped
            .into_iter()
            .map(|i| {
                let i = i.expect("every source action is mapped");
                Destination { index: i, created: created[i] }
            })
            .collect(),
    ))
}

/// Walks one combo's probability/EV row through a node's [`destination_map`] output exactly once,
/// merging mass onto the legalized menu and deciding each destination's EV: a destination that was
/// itself one of the source's own legal actions keeps only its **own** normalized EV (never
/// averaged with a moved source's payoff); a destination created purely by a move gets no EV
/// (`None`) and is flagged `Unavailable::MovedProbability`; two source actions that independently
/// collide onto the same non-created destination (e.g. two differently-sized wagers that happen
/// to round to the same live chip amount) have no unique payoff between them, so the EV is omitted
/// with a note instead of picking one arbitrarily.
pub fn legalize_row(menu: &[Action], map: &[Destination], probs: &[f32], evs: &[Option<f32>], notes: &mut Vec<String>) -> MappedAdvice {
    let mut out: Vec<MappedAction> =
        menu.iter().map(|a| MappedAction { action: *a, probability: 0.0, ev_chips: None, unavailable: None }).collect();
    let mut own_ev: Vec<Option<f32>> = vec![None; menu.len()];
    let mut ev_conflict = vec![false; menu.len()];
    for (k, d) in map.iter().enumerate() {
        merge_probability(&mut out[d.index], &menu[d.index], probs[k], d.created, notes);
        if !d.created {
            // Only a destination that was itself a source action keeps an EV, and only its own:
            // two identically rounded (or otherwise colliding) source actions have no unique
            // payoff, so the EV is omitted with a move note.
            match own_ev[d.index] {
                None => own_ev[d.index] = evs[k],
                Some(prev) if Some(prev) != evs[k] => ev_conflict[d.index] = true,
                _ => {}
            }
        }
    }
    for i in 0..menu.len() {
        if !ev_conflict[i] && out[i].unavailable.is_none() {
            out[i].ev_chips = own_ev[i];
        } else if ev_conflict[i] {
            notes.push(format!("Two rounded sources collide on {:?}; EV omitted", menu[i]));
        }
    }
    MappedAdvice { actions: out, notes: std::mem::take(notes), unsupported: None }
}

/// Merges one source action's probability mass onto its legalized `destination`, flagging a
/// created destination as `Unavailable::MovedProbability` (no EV) and recording a note either way.
pub fn merge_probability(destination: &mut MappedAction, from: &Action, probability: f32, created: bool, notes: &mut Vec<String>) {
    destination.probability += probability;
    if created {
        destination.ev_chips = None;
        destination.unavailable = Some(Unavailable::MovedProbability { from: *from });
    }
    notes.push(format!("Moved {from:?} probability {probability} to {:?}", destination.action));
}
