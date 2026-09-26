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

/// The one shared fold-consistency predicate (P3.T9 fix round 1, R2), evaluated entirely in
/// `f64`: `true` for a missing fold value or an unverified reference (neither is evidence of a
/// contradiction), else the same `1e-3` cross-check `verify_fold` states, computed at wire
/// precision. [`verify_fold`] (the narrowed `f32` API this module's public interface requires,
/// used for in-memory validation -- e.g. at [`expand_node`]'s boundary assertion) and
/// `crate::store::check_fold_consistency_wide` (the load-time admission gate, run on the
/// ORIGINAL wire-precision `f64` EV cell before `decode` ever narrows anything) both delegate to
/// this one function rather than keeping two divergent copies of the cross-check arithmetic.
pub(crate) fn verify_fold_wide(reference: EvReference, fold: Option<f64>, committed: f64, start: f64) -> bool {
    match (reference, fold) {
        (EvReference::Unverified, _) | (_, None) => true,
        (EvReference::DecisionIncrementalVerified, Some(v)) => v.abs() <= 1e-3,
        (EvReference::NetHandStartVerified, Some(v)) => (v + committed).abs() <= 1e-3,
        (EvReference::AbsoluteStackVerified, Some(v)) => (v - (start - committed)).abs() <= 1e-3,
    }
}

/// Whether a *present* fold EV is self-consistent with `committed`/`start` under `reference`,
/// within section 8.3's `1e-3` cross-check tolerance. `true` for a missing fold value or an
/// unverified reference: neither is evidence of a contradiction -- a source is not required to
/// publish fold's EV at all, and an unverified source's numbers are never checked against
/// anything.
///
/// A thin `f32` wrapper over [`verify_fold_wide`] (P3.T9 fix round 1, R2): widens its inputs and
/// delegates, so this and the load-time wide admission gate are one predicate, never two that can
/// drift apart. Because this widens an *already-narrowed* `f32` value, it can disagree with the
/// wide gate right at the `1e-3` boundary (narrowing can shift a residual by less than a part in
/// `1e7`) -- that disagreement is exactly why the wide gate, not this function, is the load-time
/// admission decision; this narrowed form remains for in-memory/direct-construction validation,
/// where no wire-precision value is available at all.
pub fn verify_fold(reference: EvReference, fold: Option<f32>, committed: f32, start: f32) -> bool {
    verify_fold_wide(reference, fold.map(f64::from), committed as f64, start as f64)
}

/// The shared checked chip conversion (P3.T9 fix round 1, R1): computes `inc_sb * 0.5 * unit` in
/// `f64` and checks the narrowed result is still a finite `f32` before returning it. `None` --
/// never a clamp, never a zero -- for a result that does not fit: an admitted, merely-finite
/// source EV (section 8.3 bounds only *that* domain) is not guaranteed to stay finite once
/// multiplied by `0.5 * unit`.
fn checked_ev_chips(inc_sb: f32, unit: u32) -> Option<f32> {
    let wide = inc_sb as f64 * 0.5 * unit as f64;
    let narrowed = wide as f32;
    narrowed.is_finite().then_some(narrowed)
}

/// A normalized EV in source-SB units, converted to chips at `unit` chips per source unit: one
/// source SB is half a source unit, since `source_blinds == [0.5, 1.0]` always (spec section 8.2).
///
/// # Panics
/// Panics (in every build profile, per the standing ruling: an infallible-signature helper
/// enforces its invariant with an always-on `assert!`) if the conversion does not fit a finite
/// `f32` -- callers that must not panic on an oversized EV (namely [`expand_node`], since a
/// public source can carry an arbitrary finite value) use [`checked_ev_chips`] directly and
/// propagate the failure as `None` instead (P3.T9 fix round 1, R1).
pub fn ev_chips(inc_sb: f32, unit: u32) -> f32 {
    checked_ev_chips(inc_sb, unit)
        .unwrap_or_else(|| panic!("ev_chips: {inc_sb} * 0.5 * {unit} does not fit a finite f32"))
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
/// value: `crate::store::check_fold_consistency_wide` rejects, at load time (inside
/// `checked_envelope`, on the ORIGINAL wire-precision EV cells), any bundle whose declared fold EV
/// fails the shared cross-check ([`verify_fold_wide`]/[`verify_fold`]), and this function's own
/// boundary assertion (P3.T9 fix round 1, R3 -- see [`check_fold_consistency`]) re-checks the same
/// invariant on whatever `node`/`info` it was actually given, for callers that bypassed the loader
/// entirely -- but only when `node.fold_wide_verified` is `false` (P3.T9 fix round 2, N1): a node
/// the loader already admitted through the wide gate is trusted as-is, never re-decided from its
/// own narrowed `f32` cell (see the assertion's own comment below for why). Either way, by the
/// time this line runs, every fold EV is already known consistent with `committed`/`start` to
/// within the spec's `1e-3` tolerance. Forcing the exact value is what section 8.3's "verified
/// fold ... store normalized fold as exact 0.0" means: a source's own rounding is never allowed to
/// leak a nonzero residual into the one action the root contract (section 6) defines as exactly
/// zero. This is the one place this function's behavior goes beyond the brief's literal Step 4
/// code (which applies `normalize_ev`/`ev_chips` uniformly to every action) -- see the task report
/// for why the addition is necessary rather than optional polish.
pub fn expand_node(node: &PreflopNode, info: &BundleInfo, actor: Seat, unit: u32, actor_max_to: u32) -> ExpandedNode {
    let classes = info.combo_classes(); // the process-wide table, borrowed, never rebuilt or cloned here
    let charts = matches!(info.source, SourceKind::ChartTranscription);
    // R3 (P3.T9 fix round 1), refined by N1 (fix round 2): before any verified, present fold EV
    // can be forced to exactly 0.0 below, this re-checks fold consistency on `node`/`info` as
    // they actually are at this call -- but ONLY when `node.fold_wide_verified` is false. A node
    // stamped `true` already passed `check_fold_consistency_wide` (the load-time gate, run on the
    // ORIGINAL `f64` wire cells) inside `checked_envelope`; re-deriving a *narrow* (`f32`)
    // residual from that same, already-narrowed node here can disagree with the wide gate purely
    // from rounding right at the `1e-3` boundary (the re-review's own repro: wire 195.0009999
    // under `AbsoluteStackVerified` has a wide residual of 0.0009999, which accepts, but narrows
    // to a residual of ~0.0010071, which would reject) -- panicking on data the loader already
    // admitted. `PreflopNode`/`BundleInfo` are publicly constructible and `PreflopStore::
    // from_sources` accepts a source without ever running the loader's admission gate at all, so
    // a node built that way carries `fold_wide_verified: false` and still gets this narrow,
    // in-memory check as its only safety net -- an always-on assertion, naming the offending
    // class. Skipped entirely, either way, where the fold-forcing branch below is also skipped
    // (charts and `Unverified` never reach it), so it never fires spuriously on data this
    // function was never going to trust anyway.
    if !charts && info.ev_reference != EvReference::Unverified && !node.fold_wide_verified {
        if let Err(msg) = check_fold_consistency(node, info) {
            panic!("expand_node: fold-EV consistency invariant violated: {msg}");
        }
    }
    let actions: Vec<Action> = node.actions.iter().map(|s| to_chip_action(s, unit, actor_max_to)).collect();
    let cols: Vec<Vec<f32>> = (0..node.actions.len()).map(|a| expand_column(classes, &node.probs, a)).collect();
    let evs: Vec<Vec<Option<f32>>> = (0..node.actions.len())
        .map(|a| {
            if charts || info.ev_reference == EvReference::Unverified {
                return vec![None; 1326];
            }
            let is_fold = matches!(node.actions[a], PreflopStep::Fold);
            let src = node.ev_source_sb.as_ref();
            let col = src.map(|rows| expand_optional(classes, rows, a)).unwrap_or_else(|| vec![None; 1326]);
            col.into_iter()
                .map(|v| {
                    normalize_ev(info.ev_reference, v, node.committed_by_actor_sb, info.source_stack_sb())
                        // R1 (P3.T9 fix round 1): the checked conversion, never the infallible
                        // `ev_chips`, so an unrepresentable chip EV propagates as `None` instead
                        // of panicking or silently becoming `inf`/a fabricated zero.
                        .and_then(|x| if is_fold { Some(0.0) } else { checked_ev_chips(x, unit) })
                })
                .collect()
        })
        .collect();
    let unreachable = expand_mask(classes, &node.unreachable);
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

/// The narrowed, in-memory fold-consistency check (spec section 8.3's cross-check, `1e-3`
/// tolerance) at every hand class that declares a fold EV, over an already-constructed
/// `PreflopNode`/`BundleInfo` pair -- necessarily on `f32` cells, since that is all a
/// `PreflopNode` ever stores.
///
/// P3.T9 fix round 1 (R2/R3): this is **not** the load-time admission decision. That gate is
/// `crate::store::check_fold_consistency_wide`, run inside `checked_envelope` on the ORIGINAL
/// wire-precision (`f64`) EV cells before `decode` narrows anything -- a bundle that fails there
/// is quarantined through the usual `load_bundle` -> `load_contained_bundle` ->
/// `PreflopStore::open` path, and a bundle that passes is never re-rejected here (a second,
/// narrow-only check at that stage could disagree with the wide one purely from `f32` rounding
/// right at the `1e-3` boundary, which is the exact defect R2 fixes). This function instead backs
/// [`expand_node`]'s own boundary assertion (R3): `PreflopNode`/`BundleInfo` are publicly
/// constructible and `PreflopStore::from_sources` accepts a source without ever running the
/// loader, so `expand_node` re-checks fold consistency itself, on whatever it was actually given,
/// before forcing a verified present fold to exactly `0.0` -- an always-on `assert`-style panic,
/// naming the offending class, rather than silently trusting an invariant no admission path
/// enforced.
///
/// P3.T9 fix round 2 (N1): `expand_node` now calls this only when `node.fold_wide_verified` is
/// `false` -- i.e. only for a node that never passed the wide gate in the first place. Calling
/// this narrow check *again* on a node that already carries `fold_wide_verified: true` is exactly
/// the defect N1 identifies: a narrow-only re-check right at the `1e-3` boundary can disagree with
/// the wide gate that already admitted the bundle, purely from `f32` rounding, and panic on
/// otherwise-valid, loader-admitted data. So this function's role narrows to backing the
/// direct-construction safety net alone, never a second opinion on a node the loader already
/// vouched for.
///
/// A missing fold action, a missing EV column, a missing per-class value, an unverified reference
/// or a chart source (which never carries EV data at all) are all silently fine -- [`verify_fold`]
/// already returns `true` for every one of them.
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
    /// never on any per-bundle data, so every caller **borrows** the one process-wide table
    /// instead of rebuilding or cloning it.
    ///
    /// P3.T9 fix round 1 (R5): returns `&'static ComboClasses`, not an owned clone, and
    /// `crate::store::load_bundle` calls this once at bundle admission (right after the
    /// manifest deserializes, before the node bytes are even read) so the table is built at
    /// load time -- not deferred to whichever bundle happens to be expanded first. Every later
    /// call, from any bundle, from `expand_node`, or from this eager admission call itself,
    /// returns the exact same `'static` table.
    pub(crate) fn combo_classes(&self) -> &'static ComboClasses {
        static CLASSES: std::sync::OnceLock<ComboClasses> = std::sync::OnceLock::new();
        CLASSES.get_or_init(ComboClasses::build)
    }

    /// The source stack in source SB units at this bundle's own depth, in `f64` (P3.T9 fix round
    /// 1, R2): `source_blinds == [0.5, 1.0]` always (checked at load), so one source BB is
    /// exactly two source SB. `depth_bb` is a `u16`, exactly representable in both `f32` and
    /// `f64`, so this and [`BundleInfo::source_stack_sb`] never disagree -- but the load-time wide
    /// fold-EV admission gate (`crate::store::check_fold_consistency_wide`) uses this `f64` form
    /// directly, never routing through the narrowed one first, to stay consistent with the
    /// wide-before-narrow rule applied to every other quantity in that check.
    pub(crate) fn source_stack_sb_f64(&self) -> f64 {
        2.0 * self.depth_bb as f64
    }

    /// The narrowed `f32` form of [`BundleInfo::source_stack_sb_f64`], for [`expand_node`] and
    /// every other in-memory (`f32`-domain) computation.
    pub(crate) fn source_stack_sb(&self) -> f32 {
        self.source_stack_sb_f64() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P3.T9 fix round 1, R5: `combo_classes` shares one process-wide table rather than handing
    /// back a fresh clone -- two different `BundleInfo`s (and repeated calls on the same one)
    /// must all borrow the identical `'static` allocation. `combo_classes` is `pub(crate)`, so
    /// this white-box check lives inside the crate rather than in `tests/ev.rs`.
    #[test]
    fn combo_classes_is_a_shared_static_table_not_a_fresh_clone() {
        fn info(bundle_id: &str) -> BundleInfo {
            BundleInfo {
                bundle_id: bundle_id.into(),
                source: SourceKind::PokerDataJson,
                depth_bb: 100,
                depths: vec![100],
                source_blinds: [0.5, 1.0],
                rake_profile: "test".into(),
                rake: None,
                straddle: false,
                version: 2,
                game: "nl".into(),
                ev_unit: "source_sb".into(),
                ev_reference: EvReference::Unverified,
                license_note: "test".into(),
                accuracy: "unverified".into(),
                sha256: String::new(),
            }
        }
        let a = info("a");
        let b = info("b");
        let ra = a.combo_classes();
        let rb = b.combo_classes();
        let ra_again = a.combo_classes();
        assert!(std::ptr::eq(ra, rb), "two different bundles must borrow the same static table");
        assert!(std::ptr::eq(ra, ra_again), "repeated calls must borrow the same static table, never a fresh clone");
    }
}
