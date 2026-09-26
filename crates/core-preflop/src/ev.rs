//! EV-reference normalization and 169-class -> 1326-combo expansion (spec section 8.3's four EV
//! formulas and section 4.1's combo classes).
//!
//! The source's own declared EV values are meaningless on their own: `DecisionIncrementalVerified`
//! is already the app's zero-at-decision-point convention (section 2's "chips already in the pot
//! are sunk"), but `NetHandStartVerified` and `AbsoluteStackVerified` are relative to a different
//! baseline and must be shifted onto that same convention before anything downstream (frequency
//! display, headline EV, replay) can compare or sum them. `Unverified` is not a baseline at all --
//! its numbers carry no known relationship to chips, so they are never surfaced.

use crate::envelope::{BundleInfo, EvReference, PreflopNode, PreflopStep, SourceKind};
use proto::{Action, Seat};

/// The four EV-reference formulas (spec section 8.3), returning the incremental, decision-point
/// EV in source-SB units -- `None` for a missing value or an unverified reference, never a
/// fabricated zero.
///
/// `committed` is the actor's own contribution so far this prefix (including posts), in source SB;
/// `start` is the source stack in source SB ([`BundleInfo::source_stack_sb`]).
pub fn normalize_ev(reference: EvReference, value: Option<f32>, committed: f32, start: f32) -> Option<f32> {
    let v = value?;
    let out = match reference {
        EvReference::DecisionIncrementalVerified => v,
        EvReference::NetHandStartVerified => v + committed,
        EvReference::AbsoluteStackVerified => v - (start - committed),
        EvReference::Unverified => return None,
    };
    out.is_finite().then_some(out)
}

/// Whether a *present* fold EV is self-consistent with `committed`/`start` under `reference`,
/// within section 8.3's `1e-3` cross-check tolerance. `true` for a missing fold value or an
/// unverified reference: neither is evidence of a contradiction -- a source is not required to
/// publish fold's EV at all, and an unverified source's numbers are never checked against
/// anything.
pub fn verify_fold(reference: EvReference, fold: Option<f32>, committed: f32, start: f32) -> bool {
    match (reference, fold) {
        (EvReference::Unverified, _) | (_, None) => true,
        (EvReference::DecisionIncrementalVerified, Some(v)) => v.abs() <= 1e-3,
        (EvReference::NetHandStartVerified, Some(v)) => (v + committed).abs() <= 1e-3,
        (EvReference::AbsoluteStackVerified, Some(v)) => (v - (start - committed)).abs() <= 1e-3,
    }
}

/// A normalized EV in source-SB units, converted to chips at `unit` chips per source unit: one
/// source SB is half a source unit, since `source_blinds == [0.5, 1.0]` always (spec section 8.2).
pub fn ev_chips(inc_sb: f32, unit: u32) -> f32 {
    inc_sb * 0.5 * unit as f32
}

/// combo -> class, built once from the 169 indicator expansions (spec section 4.1's 6/4/12
/// orbits) -- every later expansion in this module borrows the same table rather than rebuilding
/// it per column.
#[derive(Clone, Debug, PartialEq)]
pub struct ComboClasses {
    class_of: [u16; 1326],
}

impl ComboClasses {
    /// # Panics
    /// Panics (in every build profile, per the standing ruling: internal constructors with
    /// infallible signatures enforce their invariants with always-on `assert!`, naming the
    /// offending index) if any combo is left unassigned, naming the first such combo index --
    /// every one of the 1326 combos belongs to exactly one of the 169 classes (spec section 4.1),
    /// so this can only fire if `core_ranges::expand_169`'s own class partition stops covering
    /// every combo.
    ///
    /// The task brief writes this invariant check as `debug_assert!`; the project's standing
    /// ruling on infallible-constructor invariants requires an always-on `assert!` instead, so
    /// this is implemented as `assert!` (a plan-mandated deviation from the brief's literal code,
    /// not from its signature).
    pub fn build() -> Self {
        let mut class_of = [u16::MAX; 1326];
        for c in 0..169 {
            let mut indicator = [0.0f32; 169];
            indicator[c] = 1.0;
            let mask = core_ranges::expand_169(&indicator); // a valid Range1326
            for (i, m) in mask.0.iter().enumerate() {
                if *m != 0.0 {
                    class_of[i] = c as u16;
                }
            }
        }
        if let Some(i) = class_of.iter().position(|&c| c == u16::MAX) {
            panic!("ComboClasses::build: combo {i} was not assigned a class by any of the 169 indicator expansions");
        }
        Self { class_of }
    }

    pub fn class(&self, combo: usize) -> usize {
        self.class_of[combo] as usize
    }
}

/// Column `a` of a class-major `[169][actions]` matrix, expanded to 1326 combos.
pub fn expand_column(classes: &ComboClasses, rows: &[Vec<f32>], a: usize) -> Vec<f32> {
    (0..1326).map(|i| rows[classes.class(i)][a]).collect()
}

/// [`expand_column`]'s `Option<f32>` twin, for EV columns. Never passes the EV payload through a
/// range validator -- EVs can be negative or above 1, unlike a range weight; only
/// [`ComboClasses::build`] feeds `expand_169` a legal indicator.
pub fn expand_optional(classes: &ComboClasses, rows: &[Vec<Option<f32>>], a: usize) -> Vec<Option<f32>> {
    (0..1326).map(|i| rows[classes.class(i)][a]).collect()
}

/// A class-indexed unreachable mask, expanded to 1326 combos: every combo of an unreachable class
/// is unreachable too.
pub fn expand_mask(classes: &ComboClasses, unreachable: &[bool; 169]) -> Vec<bool> {
    (0..1326).map(|i| unreachable[classes.class(i)]).collect()
}

/// One preflop node expanded from 169 hand classes to 1326 combos, in chip-domain actions and
/// chip-domain EVs (spec sections 8.3/8.4's boundary: everything before this point is class-major
/// source units, everything from here is combo-major chips, until Task 10's legality mapping).
#[derive(Clone, Debug, PartialEq)]
pub struct ExpandedNode {
    pub actor: Seat,
    pub actions: Vec<Action>,
    pub probs: Vec<Vec<f32>>,
    pub ev_chips: Vec<Vec<Option<f32>>>,
    pub available: Vec<bool>,
    pub ev_reference: EvReference,
    pub source: SourceKind,
}

/// One source step converted to a chip-domain action at `unit` chips per source unit: a raise's
/// `to_bb_x1000` (thousandths of a source unit) rounds to the nearest chip, half up
/// (`(to_bb_x1000 * unit + 500) / 1000`, computed exactly in `u64` before the checked narrowing);
/// `AllIn` carries no source size at all and becomes the actor's own actual maximum, `actor_max_to`
/// -- never the source's declared depth, which can be shallower or deeper than the live stack.
/// Legality against the live tree is Task 10's job, not this conversion's.
///
/// # Panics
/// Panics (in every build profile) if a raise's rounded chip amount does not fit `u32` -- the
/// source-key domain (`u32` thousandths of a unit) is already narrower than a raise that would
/// not fit here, so this can only fire at an unrealistically large `unit`.
fn to_chip_action(step: &PreflopStep, unit: u32, actor_max_to: u32) -> Action {
    match step {
        PreflopStep::Fold => Action::Fold,
        PreflopStep::Check => Action::Check,
        PreflopStep::Call => Action::Call,
        PreflopStep::Raise { to_bb_x1000 } => {
            let milli = u64::from(*to_bb_x1000) * u64::from(unit) + 500;
            let to = u32::try_from(milli / 1000).unwrap_or_else(|_| {
                panic!("to_chip_action: a {to_bb_x1000}-thousandths raise at a {unit}-chip unit does not fit u32")
            });
            Action::Raise { to }
        }
        PreflopStep::AllIn => Action::AllIn { to: actor_max_to },
    }
}

/// The full class-major-to-combo-major, source-unit-to-chip expansion of one selected source node
/// (spec sections 8.3/8.4): probabilities and EVs move from `[169][actions]` to `[1326][actions]`,
/// EVs move from source SB to chips through [`normalize_ev`]/[`ev_chips`], and the unreachable mask
/// becomes a combo-indexed `available` flag. No board blocking happens here (section 8.4's job);
/// legality after chip conversion is Task 10's.
///
/// A *present* fold EV always normalizes to exactly `0.0` chips here, never a residual near-zero
/// value: [`crate::store::build_node_map`] rejects, at load time, any bundle whose declared fold EV
/// fails [`verify_fold`]'s cross-check (this task's Step 5 -- see [`check_fold_consistency`]), so
/// every fold EV this function ever sees is already known consistent with `committed`/`start` to
/// within the spec's `1e-3` tolerance. Forcing the exact value is what section 8.3's "verified fold
/// ... store normalized fold as exact 0.0" means: a source's own rounding is never allowed to leak
/// a nonzero residual into the one action the root contract (section 6) defines as exactly zero.
/// This is the one place this function's behavior goes beyond the brief's literal Step 4 code
/// (which applies `normalize_ev`/`ev_chips` uniformly to every action) -- see the task report for
/// why the addition is necessary rather than optional polish.
pub fn expand_node(node: &PreflopNode, info: &BundleInfo, actor: Seat, unit: u32, actor_max_to: u32) -> ExpandedNode {
    let classes = info.combo_classes(); // the table built once at bundle load
    let charts = matches!(info.source, SourceKind::ChartTranscription);
    let actions: Vec<Action> = node.actions.iter().map(|s| to_chip_action(s, unit, actor_max_to)).collect();
    let cols: Vec<Vec<f32>> = (0..node.actions.len()).map(|a| expand_column(&classes, &node.probs, a)).collect();
    let evs: Vec<Vec<Option<f32>>> = (0..node.actions.len())
        .map(|a| {
            if charts || info.ev_reference == EvReference::Unverified {
                return vec![None; 1326];
            }
            let is_fold = matches!(node.actions[a], PreflopStep::Fold);
            let src = node.ev_source_sb.as_ref();
            let col = src.map(|rows| expand_optional(&classes, rows, a)).unwrap_or_else(|| vec![None; 1326]);
            col.into_iter()
                .map(|v| {
                    normalize_ev(info.ev_reference, v, node.committed_by_actor_sb, info.source_stack_sb())
                        .map(|x| if is_fold { 0.0 } else { ev_chips(x, unit) })
                })
                .collect()
        })
        .collect();
    let unreachable = expand_mask(&classes, &node.unreachable);
    ExpandedNode {
        actor,
        actions,
        probs: (0..1326).map(|c| cols.iter().map(|col| col[c]).collect()).collect(),
        ev_chips: (0..1326).map(|c| evs.iter().map(|col| col[c]).collect()).collect(),
        available: unreachable.iter().map(|&u| !u).collect(),
        ev_reference: info.ev_reference,
        source: info.source,
    }
}

/// The one node-admission check beyond structural validation that this task adds: a *present*,
/// verified fold reference must be self-consistent (spec section 8.3's cross-check, `1e-3`
/// tolerance) at every hand class that declares one. [`crate::store::build_node_map`] calls this
/// for every node and rejects the whole node -- and so, through the same `?` propagation
/// `build_node_map`'s structural checks already use, the whole bundle -- on the first inconsistent
/// class, naming it. That failure reaches the store through the same path a structural failure
/// already takes (`load_bundle` -> `load_contained_bundle` -> `PreflopStore::open`'s quarantine
/// loop), so a fold-inconsistent bundle is quarantined with a banner exactly like a malformed one,
/// while every sibling bundle -- validated on its own bytes -- stays active (this task's Step 5).
///
/// A missing fold action, a missing EV column, a missing per-class value, an unverified reference
/// or a chart source (which never carries EV data at all, already rejected earlier at
/// `checked_envelope`) are all silently fine -- [`verify_fold`] already returns `true` for every
/// one of them.
pub(crate) fn check_fold_consistency(node: &PreflopNode, info: &BundleInfo) -> Result<(), String> {
    let Some(fold_idx) = node.actions.iter().position(|s| matches!(s, PreflopStep::Fold)) else {
        return Ok(());
    };
    let Some(evs) = node.ev_source_sb.as_ref() else {
        return Ok(());
    };
    for (c, row) in evs.iter().enumerate() {
        let fold_ev = row[fold_idx];
        if !verify_fold(info.ev_reference, fold_ev, node.committed_by_actor_sb, info.source_stack_sb()) {
            return Err(format!(
                "class {c}'s fold EV {fold_ev:?} is inconsistent with committed {} under {:?} (tolerance 1e-3)",
                node.committed_by_actor_sb, info.ev_reference
            ));
        }
    }
    Ok(())
}

impl BundleInfo {
    /// The combo -> 169-class table (spec section 4.1), computed once per process and shared by
    /// every bundle: the mapping depends only on `core_ranges::expand_169`'s fixed class order,
    /// never on any per-bundle data, so every call after the first clones the cached table instead
    /// of rebuilding it ("the table built once at bundle load", per the brief).
    pub(crate) fn combo_classes(&self) -> ComboClasses {
        static CLASSES: std::sync::OnceLock<ComboClasses> = std::sync::OnceLock::new();
        CLASSES.get_or_init(ComboClasses::build).clone()
    }

    /// The source stack in source SB units at this bundle's own depth: `source_blinds ==
    /// [0.5, 1.0]` always (checked at load), so one source BB is exactly two source SB.
    pub(crate) fn source_stack_sb(&self) -> f32 {
        2.0 * self.depth_bb as f32
    }
}
