//! Result assembly (spec sections 4.4, 5, 6 and 8.4): the one place a `Recommendation` is put
//! together from a solved node or known masses, a coverage label and the equity summary, and the
//! one headline rule every path uses (plan 3 extends the call sites, never adds a second entry point).
//!
//! Standing rules carried here:
//! * reasons accumulate across source, replay, cache and solve, without duplicates, and are never
//!   removed: `Exact` becomes `Approximate` as soon as one reason exists, an `Approximate` never
//!   becomes `Exact` again, and an `Unsupported` outcome keeps what was accumulated in `partial`;
//! * EV is signed finite chips, `ev_bb = ev_chips / bb_chips`, fold is exactly 0 (chips already in
//!   the pot are sunk);
//! * there is no headline of any kind while `unresolved_mass > 0` (spec 8.4);
//! * a `Ready` equity estimate is never replaced by a `Pending` one.
//!
//! Every function here has an infallible signature, so its preconditions are enforced with
//! always-on `assert!`s that name the offending action, node or combo (plan-1 standing ruling b).

use proto::worker::NodeStrategy;
use proto::*;

/// What assembly needs about the decision that is not in the node strategy.
#[derive(Clone, Debug)]
pub struct AssemblyCtx {
    pub identity: DecisionIdentity,
    /// `Derived.legal`: the legal intervals, always shown, and the chips hero's advice is mapped to.
    pub legal: Vec<LegalAction>,
    /// Hero's actual combo. On record at every decision point (spec section 2); `None` only while a
    /// context is used for `fast` or `unsupported`, which never read it.
    pub hero_combo: Option<ComboIndex>,
    pub bb_chips: u32,
    pub equity: EquitySummary,
}

/// Where the frequencies behind a frequency headline came from (spec 4.4 rule 2's three wordings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlineSource { Solved, Chart, PokerDataUnverified }

/// Every reason of `base` and then of `more`, each once, in first-seen order. Each reason is also
/// checked against its wire codecs here, where it enters a coverage label, rather than later at the
/// IPC boundary, where a non-finite or out-of-domain field would make the whole event unsendable.
fn union(base: Vec<ApproxReason>, more: Vec<ApproxReason>) -> Vec<ApproxReason> {
    let mut out: Vec<ApproxReason> = Vec::with_capacity(base.len() + more.len());
    for (i, r) in base.into_iter().chain(more).enumerate() {
        if let Err(e) = serde_json::to_value(&r) { panic!("accumulate: reason {i} ({r:?}) is not a valid wire value: {e}"); }
        if !out.contains(&r) { out.push(r); }
    }
    out
}

/// Spec 4.4 / 6: reasons accumulate. An `Exact` base with reasons becomes `Approximate`; an
/// `Approximate` base stays `Approximate` even with nothing to add (a later exact result never
/// removes inherited reasons); an `Unsupported` base accumulates into `partial`. Duplicates (by
/// value) are dropped after their first occurrence; nothing else ever is.
pub fn accumulate(base: Coverage, more: Vec<ApproxReason>) -> Coverage {
    match base {
        Coverage::Exact => {
            let reasons = union(Vec::new(), more);
            if reasons.is_empty() { Coverage::Exact } else { Coverage::Approximate { reasons } }
        }
        Coverage::Approximate { reasons } => Coverage::Approximate { reasons: union(reasons, more) },
        Coverage::Unsupported { reason, partial } => Coverage::Unsupported { reason, partial: union(partial, more) },
    }
}

/// Spec 5 step 7: `Exact` iff raw `exploitability_chips / pot <= target_bp / 10000` and no reason
/// at all; otherwise `Approximate` with the inherited reasons, plus `DeadlineBestSoFar` when the solve
/// was stopped by its deadline (`best_so_far`).
///
/// The comparison is raw, never on rounded basis points (spec 4.4), and exact: it is
/// `proto::worker::meets_target` (`exploitability * 10_000 <= target_bp * pot` in `f64`, where both
/// products are exact), the one definition the solve client, the bench and the worker share (final
/// review M3). `reached_bp` is display only; a value above `u16::MAX` bp (6.5535 times the pot, or
/// 655.35%) is shown as `u16::MAX`, and no comparison ever reads it.
///
/// # Panics
/// If `pot` is 0 or `exploitability_chips` is not finite and non-negative (the worker's result is
/// validated before it gets here).
pub fn coverage_for_solve(exploitability_chips: f32, pot: u32, target_bp: u16, best_so_far: bool, inherited: Vec<ApproxReason>) -> Coverage {
    assert!(pot > 0, "coverage_for_solve: the solved pot is 0 chips");
    assert!(exploitability_chips.is_finite() && exploitability_chips >= 0.0, "coverage_for_solve: exploitability {exploitability_chips} chips is not finite and non-negative");
    let raw = f64::from(exploitability_chips);
    let at_target = proto::worker::meets_target(exploitability_chips, pot, target_bp);
    let mut reasons = inherited;
    if best_so_far {
        let bp = (raw * 10_000.0 / f64::from(pot)).round();
        let reached_bp = if bp <= f64::from(u16::MAX) { bp as u16 } else { u16::MAX };
        reasons.push(ApproxReason::DeadlineBestSoFar { reached_bp, target_bp });
    }
    if at_target && reasons.is_empty() { Coverage::Exact } else { Coverage::Approximate { reasons: union(Vec::new(), reasons) } }
}

/// Spec 4.4's headline rule, the only implementation of it. Clears every `headline` flag, then:
/// 1. every action has `ev_bb` -> the highest-EV action (ties: higher frequency, then earlier in menu
///    order), labelled "highest EV";
/// 2. otherwise no EV ranking at all; every action has `frequency` -> the highest-frequency action
///    (ties: earlier in menu order), labelled "highest-frequency action, EV incomplete" when some
///    action carries `BranchSupportIncomplete`, else by `source`: "highest-frequency chart action"
///    (`Chart`), "highest-frequency source action, EV reference unverified" (`PokerDataUnverified`),
///    and no headline for `Solved` (a solve that lacks an EV has no source wording);
/// 3. otherwise, and whenever `unresolved_mass > 0`, no headline of any kind.
///
/// # Panics
/// If `unresolved_mass` is outside `[0, 1]`, or an action's frequency is outside `[0, 1]` or its
/// `ev_bb` is not finite (naming the action).
pub fn headline(actions: &mut [ActionAdvice], unresolved_mass: f32, source: HeadlineSource) -> Option<String> {
    assert!(unresolved_mass.is_finite() && (0.0..=1.0).contains(&unresolved_mass), "headline: unresolved mass {unresolved_mass} is outside [0, 1]");
    for (i, a) in actions.iter().enumerate() {
        if let Some(f) = a.frequency { assert!(f.is_finite() && (0.0..=1.0).contains(&f), "headline: action {i} ({:?}) has frequency {f} outside [0, 1]", a.action); }
        if let Some(ev) = a.ev_bb { assert!(ev.is_finite(), "headline: action {i} ({:?}) has a non-finite ev_bb {ev}", a.action); }
    }
    for a in actions.iter_mut() { a.headline = false; }
    if actions.is_empty() || unresolved_mass > 0.0 { return None; }
    if actions.iter().all(|a| a.ev_bb.is_some()) {
        let ev = |a: &ActionAdvice| a.ev_bb.expect("checked: every action has ev_bb");
        let mut best = 0;
        for i in 1..actions.len() {
            let (x, b) = (ev(&actions[i]), ev(&actions[best]));
            if x > b || (x == b && actions[i].frequency > actions[best].frequency) { best = i; }
        }
        actions[best].headline = true;
        return Some("highest EV".into());
    }
    if actions.iter().all(|a| a.frequency.is_some()) {
        let incomplete = actions.iter().any(|a| matches!(a.unavailable, Some(Unavailable::BranchSupportIncomplete { .. })));
        let label = if incomplete {
            "highest-frequency action, EV incomplete"
        } else {
            match source {
                HeadlineSource::Chart => "highest-frequency chart action",
                HeadlineSource::PokerDataUnverified => "highest-frequency source action, EV reference unverified",
                HeadlineSource::Solved => return None,
            }
        };
        let mut best = 0;
        for i in 1..actions.len() { if actions[i].frequency > actions[best].frequency { best = i; } }
        actions[best].headline = true;
        return Some(label.into());
    }
    None
}

/// Shape and value checks for a node strategy read by assembly: 1326 rows of the menu's width,
/// probabilities in `[0, 1]`, finite EVs. `validate_solution` has already checked the same on the
/// wire; this keeps the promise for any other caller.
fn check_node(what: &str, k: usize, node: &NodeStrategy) {
    let w = node.actions.len();
    assert!(node.probs.len() == COMBOS && node.ev_chips.len() == COMBOS && node.available.len() == COMBOS,
        "{what}: node {k} must have {COMBOS} probability, EV and availability rows (has {}, {}, {})", node.probs.len(), node.ev_chips.len(), node.available.len());
    for c in 0..COMBOS {
        assert!(node.probs[c].len() == w && node.ev_chips[c].len() == w, "{what}: node {k} combo {c}: rows must have the menu's {w} entries");
        for a in 0..w {
            let (p, ev) = (node.probs[c][a], node.ev_chips[c][a]);
            assert!(p.is_finite() && (0.0..=1.0).contains(&p), "{what}: node {k} combo {c} action {a}: probability {p} is outside [0, 1]");
            assert!(ev.is_finite(), "{what}: node {k} combo {c} action {a}: EV {ev} is not finite");
        }
    }
}

fn check_reach(what: &str, reach: &[f32]) {
    assert!(reach.len() == COMBOS, "{what}: reach has {} entries, expected {COMBOS}", reach.len());
    for (c, r) in reach.iter().enumerate() {
        assert!(r.is_finite() && *r >= 0.0, "{what}: reach of combo {c} is {r}, not finite and non-negative");
    }
}

/// Hero's reach at the requested node: hero's public (street-root) weights times hero's own action
/// probabilities at every earlier hero node on the path to it. Villain's actions never enter: they
/// condition villain's range, not hero's.
///
/// Only exported nodes can condition. A prefix node that is absent from the export (an
/// `export: "truncated"` result carries the requested node only, spec 4.5) contributes nothing, so
/// the caller discloses a truncated export; it is never repaired by a guessed probability.
///
/// # Panics
/// If `nodes` and `ordinal_paths` differ in length, `requested` is not a node, a hero weight is not
/// finite and non-negative, or a hero node on the path is malformed or lacks the action the path takes.
pub fn hero_reach(nodes: &[NodeStrategy], ordinal_paths: &[OrdinalPath], requested: usize, hero_public: &Range1326, hero_actor: &str) -> Vec<f32> {
    assert_eq!(nodes.len(), ordinal_paths.len(), "hero_reach: {} nodes but {} ordinal paths", nodes.len(), ordinal_paths.len());
    assert!(requested < nodes.len(), "hero_reach: requested node {requested} is not below {} nodes", nodes.len());
    check_reach("hero_reach (hero's public range)", &hero_public.0);
    let mut reach: Vec<f64> = hero_public.0.iter().map(|w| f64::from(*w)).collect();
    let target = &ordinal_paths[requested];
    for k in 0..target.len() {
        let Some(i) = ordinal_paths.iter().position(|p| p[..] == target[..k]) else { continue };
        let n = &nodes[i];
        if n.actor != hero_actor { continue; }
        check_node("hero_reach", i, n);
        let a = usize::from(target[k]);
        assert!(a < n.actions.len(), "hero_reach: the path to node {requested} takes action {a} at node {i}, which has {} actions", n.actions.len());
        for (c, r) in reach.iter_mut().enumerate() { *r *= f64::from(n.probs[c][a]); }
    }
    reach.into_iter().map(|r| r as f32).collect()
}

/// Spec 4.4's range-level mix: action frequencies over hero's public range at the node, weighted by
/// `reach`, over the combos available at the node. All zeros when no available combo has reach
/// (`final_from_solution` still shows it, since the field is present whenever a node strategy exists,
/// and adds a note that hero's range has no reach there, so the zeros are never read as a strategy).
///
/// # Panics
/// If the node or `reach` is malformed (see `check_node`, `check_reach`).
pub fn range_mix(node: &NodeStrategy, reach: &[f32]) -> Vec<(Action, f32)> {
    check_node("range_mix", 0, node);
    check_reach("range_mix", reach);
    let mut mass = 0.0f64;
    let mut acc = vec![0.0f64; node.actions.len()];
    for c in 0..COMBOS {
        if node.available[c] && reach[c] > 0.0 {
            let w = f64::from(reach[c]);
            mass += w;
            for (a, slot) in acc.iter_mut().enumerate() { *slot += w * f64::from(node.probs[c][a]); }
        }
    }
    // Each term is at most its weight, so every sum is at most `mass` and every share lies in [0, 1].
    node.actions.iter().copied().zip(acc.into_iter().map(|x| if mass > 0.0 { (x / mass) as f32 } else { 0.0 })).collect()
}

/// Maps a tree action onto `Derived.legal`: the same kind with its chips inside the legal interval.
/// Tree actions carry the tree's chips; hero's real all-in amount comes from `Derived.legal`.
pub fn map_to_legal(a: &Action, legal: &[LegalAction]) -> Option<Action> {
    match a {
        Action::Fold => legal.iter().any(|l| matches!(l, LegalAction::Fold)).then_some(*a),
        Action::Check => legal.iter().any(|l| matches!(l, LegalAction::Check)).then_some(*a),
        Action::Call => legal.iter().any(|l| matches!(l, LegalAction::Call { .. })).then_some(*a),
        Action::Bet { to } => legal.iter().find_map(|l| match l { LegalAction::Bet { min_to, max_to } if to >= min_to && to <= max_to => Some(*a), _ => None }),
        Action::Raise { to } => legal.iter().find_map(|l| match l { LegalAction::Raise { min_to, max_to } if to >= min_to && to <= max_to => Some(*a), _ => None }),
        Action::AllIn { .. } => legal.iter().find_map(|l| match l { LegalAction::AllIn { to } => Some(Action::AllIn { to: *to }), _ => None }),
    }
}

/// The zero `Assumptions` of a solver-backed result for `template_id`.
pub fn empty_assumptions(template_id: &str) -> Assumptions {
    Assumptions { template_id: template_id.into(), source: "solver-worker".into(), ..crate::assumptions_stub() }
}

fn base(ctx: &AssemblyCtx, phase: Phase, coverage: Coverage, actions: Vec<ActionAdvice>, unresolved_mass: f32, range_mix: Option<Vec<(Action, f32)>>, assumptions: Assumptions) -> Recommendation {
    Recommendation { identity: ctx.identity.clone(), phase, coverage, legal: ctx.legal.clone(), actions, unresolved_mass, range_mix, equity: ctx.equity.clone(), assumptions, experimental: None, exploit: None }
}

/// The legal menu with no advice: one entry per legal interval (a wager interval by its minimum),
/// no frequency, no EV, `u` saying why.
fn legal_menu(ctx: &AssemblyCtx, u: Option<Unavailable>) -> Vec<ActionAdvice> {
    ctx.legal
        .iter()
        .map(|l| {
            let action = match l {
                LegalAction::Fold => Action::Fold,
                LegalAction::Check => Action::Check,
                LegalAction::Call { .. } => Action::Call,
                LegalAction::Bet { min_to, .. } => Action::Bet { to: *min_to },
                LegalAction::Raise { min_to, .. } => Action::Raise { to: *min_to },
                LegalAction::AllIn { to } => Action::AllIn { to: *to },
            };
            ActionAdvice { action, frequency: None, ev_bb: None, unavailable: u.clone(), headline: false }
        })
        .collect()
}

/// The `Final` for a solved node (spec 5 step 7): hero's frequencies and EVs mapped to legal chips,
/// the range mix, and the spec 4.4 headline. When hero's combo has no reach at the node the result is
/// `Unsupported{HeroComboOutOfSupport}` with the accumulated reasons in `partial` and the range mix as
/// the only strategy output (spec 6); no per-combo number is fabricated.
///
/// # Panics
/// If `ctx.bb_chips` is 0, hero's combo is not on record or not a combo index, `coverage` is
/// `Unsupported` (that outcome is assembled by [`unsupported`]), the node or `reach` is malformed, or
/// hero's fold EV is not exactly `+0.0` chips.
pub fn final_from_solution(ctx: &AssemblyCtx, node: &NodeStrategy, reach: &[f32], coverage: Coverage, mut assumptions: Assumptions) -> Recommendation {
    assert!(ctx.bb_chips > 0, "final_from_solution: bb_chips is 0, ev_bb divides by it");
    if let Coverage::Unsupported { reason, .. } = &coverage {
        panic!("final_from_solution: a solved node's coverage is Exact or Approximate, got Unsupported {{ {reason:?} }}; use `unsupported`");
    }
    let c = usize::from(ctx.hero_combo.expect("final_from_solution: hero's combo is on record at every decision point (spec section 2)"));
    assert!(c < COMBOS, "final_from_solution: hero combo {c} is not below {COMBOS}");
    check_node("final_from_solution", 0, node);
    check_reach("final_from_solution", reach);

    // Spec 4.4: present whenever a node strategy exists (ruling 26-I2), zeros included; a zero mix is
    // disclosed as such and never presented as a normalized strategy.
    let mix = Some(range_mix(node, reach));
    if !(0..COMBOS).any(|k| node.available[k] && reach[k] > 0.0) {
        assumptions.notes.push("range mix is all zeros: hero's public range has no reach at this node, so it is not a strategy".into());
    }

    if !(node.available[c] && reach[c] > 0.0) {
        let actions = node.actions.iter().map(|a| ActionAdvice { action: map_to_legal(a, &ctx.legal).unwrap_or(*a), frequency: None, ev_bb: None, unavailable: Some(Unavailable::HeroOutOfSupport), headline: false }).collect();
        let inherited = match coverage { Coverage::Approximate { reasons } => reasons, _ => vec![] };
        let coverage = accumulate(Coverage::Unsupported { reason: UnsupportedReason::HeroComboOutOfSupport, partial: vec![] }, inherited);
        return base(ctx, Phase::Final, coverage, actions, 0.0, mix, assumptions);
    }

    let (probs, evs) = (&node.probs[c], &node.ev_chips[c]);
    for (i, a) in node.actions.iter().enumerate() {
        if *a == Action::Fold {
            assert!(evs[i].to_bits() == 0.0f32.to_bits(), "final_from_solution: hero's fold (action {i}) must be exactly 0.0 chips, chips in the pot are sunk; got {}", evs[i]);
        }
    }
    let bb = f64::from(ctx.bb_chips);
    let mut unmapped: Vec<String> = Vec::new();
    let mut actions: Vec<ActionAdvice> = node
        .actions
        .iter()
        .enumerate()
        .map(|(i, a)| match map_to_legal(a, &ctx.legal) {
            Some(mapped) => ActionAdvice { action: mapped, frequency: Some(probs[i]), ev_bb: Some((f64::from(evs[i]) / bb) as f32), unavailable: None, headline: false },
            // Spec 4.4 reserves `NotInMenu` for "not in the SOURCE's menu". A tree action outside
            // `Derived.legal` is a different situation: spec 8.4 moves its probability to the nearest
            // legal action and marks the destination `MovedProbability{from}`. That mapping is plan 4's
            // bet translation, so until it lands the action keeps its frequency, has no EV, is marked
            // `NotEvaluated` and is named in a note; it is never silently mislabelled.
            None => {
                unmapped.push(format!("{a:?}"));
                ActionAdvice { action: *a, frequency: Some(probs[i]), ev_bb: None, unavailable: Some(Unavailable::NotEvaluated), headline: false }
            }
        })
        .collect();
    if !unmapped.is_empty() {
        assumptions.notes.push(format!("tree action(s) outside the legal intervals, not translated in this build (spec 8.4 MovedProbability arrives with the flop path): {}", unmapped.join(", ")));
    }
    if let Some(label) = headline(&mut actions, 0.0, HeadlineSource::Solved) { assumptions.notes.push(format!("headline: {label}")); }
    base(ctx, Phase::Final, accumulate(coverage, vec![]), actions, 0.0, mix, assumptions)
}

/// Hero's advice as the known mass per action over hero's posterior (spec 4.4 and 8.4): the
/// assembly's own input type for a decision that may leave part of the posterior without a strategy
/// (a node present in only some positive-posterior branches, the residual, a stopped branch). It
/// carries the advice fields of `core_preflop::MixedNode` (P3.T16) one for one; that type's
/// `unsupported` verdict is not a known mass and stays outside this input, and the engine does not
/// depend on `core-preflop` before P3.T17.
///
/// - `actions`: every action with its known `frequency`; `ev_bb` only over complete branch support.
/// - `unresolved_mass`: the posterior no node covers, so `sum(frequency) + unresolved_mass = 1`.
/// - `range_mix`: the range-level mix as the source computed it.
/// - `reasons`: the source's reasons (`BranchResidual` whenever `unresolved_mass > 0`).
/// - `notes`: the source's notes, among them the rendered share "x% of the posterior has no
///   strategy" whenever `unresolved_mass > 0`.
#[derive(Clone, Debug, PartialEq)]
pub struct KnownMass {
    pub actions: Vec<ActionAdvice>,
    pub unresolved_mass: f32,
    pub range_mix: Option<Vec<(Action, f32)>>,
    pub reasons: Vec<ApproxReason>,
    pub notes: Vec<String>,
}

/// How far `sum(frequency) + unresolved_mass` may sit from 1: every share was narrowed from `f64` to
/// `f32` once (at most half an `f32` ulp, about 3e-8, each), so this admits any practical menu width
/// and still catches a dropped or duplicated action.
const KNOWN_MASS_TOLERANCE: f64 = 1e-5;

/// Spec 4.4 rule 3's rendering of the unresolved share, "x% of the posterior has no strategy", with
/// `x` at one decimal as the source writes it.
const UNRESOLVED_SHARE_NOTE: &str = "% of the posterior has no strategy";

/// Whether `note` renders `unresolved_mass` as the share note: its percentage matches the mass to the
/// one-decimal display rounding (0.05 points) plus the `f32` narrowing of the mass.
fn renders_unresolved_share(note: &str, unresolved_mass: f32) -> bool {
    note.strip_suffix(UNRESOLVED_SHARE_NOTE)
        .and_then(|pct| pct.parse::<f64>().ok())
        .is_some_and(|pct| (pct - 100.0 * f64::from(unresolved_mass)).abs() <= 0.05 + 1e-4)
}

/// The `Final` for known masses (spec 4.4 and 8.4): the known frequencies exactly as given, never
/// renormalized, `unresolved_mass` as given, the source's range mix, reasons (accumulated onto
/// `coverage`) and notes, and the spec 4.4 headline from the one [`headline`] rule, which gives no
/// headline of any kind while `unresolved_mass > 0`; the share itself is displayed through its note.
///
/// # Panics
/// If `coverage` is `Unsupported` (an `Unsupported` outcome is not a known mass), an action has no
/// frequency, the masses are outside `[0, 1]` or `sum(frequency) + unresolved_mass` is not 1 (within
/// `KNOWN_MASS_TOLERANCE`), or `unresolved_mass > 0` and no note renders that share (spec 4.4 rule 3:
/// the known frequencies are never shown as a complete recommendation).
pub fn final_from_known_mass(ctx: &AssemblyCtx, known: KnownMass, coverage: Coverage, source: HeadlineSource, mut assumptions: Assumptions) -> Recommendation {
    if let Coverage::Unsupported { reason, .. } = &coverage {
        panic!("final_from_known_mass: known masses are Exact or Approximate, got Unsupported {{ {reason:?} }}");
    }
    let KnownMass { mut actions, unresolved_mass, range_mix, reasons, notes } = known;
    assert!(unresolved_mass.is_finite() && (0.0..=1.0).contains(&unresolved_mass), "final_from_known_mass: unresolved mass {unresolved_mass} is outside [0, 1]");
    let mut total = f64::from(unresolved_mass);
    for (i, a) in actions.iter().enumerate() {
        let f = a.frequency.unwrap_or_else(|| panic!("final_from_known_mass: action {i} ({:?}) has no known frequency", a.action));
        assert!(f.is_finite() && (0.0..=1.0).contains(&f), "final_from_known_mass: action {i} ({:?}) has frequency {f} outside [0, 1]", a.action);
        total += f64::from(f);
    }
    assert!((total - 1.0).abs() <= KNOWN_MASS_TOLERANCE, "final_from_known_mass: the known frequencies and the unresolved mass sum to {total}, not 1");
    for n in notes {
        if !assumptions.notes.contains(&n) { assumptions.notes.push(n); }
    }
    if unresolved_mass > 0.0 {
        assert!(assumptions.notes.iter().any(|n| renders_unresolved_share(n, unresolved_mass)),
            "final_from_known_mass: no note renders the unresolved share {unresolved_mass} as \"x{UNRESOLVED_SHARE_NOTE}\": {:?}", assumptions.notes);
    }
    if let Some(label) = headline(&mut actions, unresolved_mass, source) { assumptions.notes.push(format!("headline: {label}")); }
    base(ctx, Phase::Final, accumulate(coverage, reasons), actions, unresolved_mass, range_mix, assumptions)
}

/// A `Final` with no strategy (spec 6: equity where computable, the visible reason): the legal menu,
/// each entry `NotEvaluated`, and the reasons accumulated so far kept in `partial`, deduplicated.
pub fn unsupported(ctx: &AssemblyCtx, reason: UnsupportedReason, partial: Vec<ApproxReason>, assumptions: Assumptions) -> Recommendation {
    let coverage = accumulate(Coverage::Unsupported { reason, partial: vec![] }, partial);
    base(ctx, Phase::Final, coverage, legal_menu(ctx, Some(Unavailable::NotEvaluated)), 0.0, None, assumptions)
}

/// Spec 5 step 5: the legal intervals, the coverage so far, no frequency or EV (`Pending`), equity
/// as the context carries it (pending until the `Equity` event).
pub fn fast(ctx: &AssemblyCtx, coverage_so_far: Coverage, assumptions: Assumptions) -> Recommendation {
    base(ctx, Phase::Fast, accumulate(coverage_so_far, vec![]), legal_menu(ctx, Some(Unavailable::Pending)), 0.0, None, assumptions)
}

/// How settled an estimate is: `Pending` < `Unavailable` < `Ready`.
fn settled(a: &Availability) -> u8 {
    match a {
        Availability::Pending => 0,
        Availability::Unavailable { .. } => 1,
        Availability::Ready => 2,
    }
}

/// Spec 4.4 event merging: an `Equity` event enriches whatever is displayed, including a `Final`
/// that arrived earlier. Per seat, an estimate replaces the displayed one only when it is at least as
/// settled, so a `Ready` estimate is never replaced by a `Pending` one (nor by an `Unavailable` one),
/// and an `Unavailable` answer is never reset to `Pending`. A seat not yet shown is added.
///
/// `per_pot_shares` follows the same rule, estimate by estimate: pots are matched by `pot_index` and
/// the shares within a pot by seat. A pot or seat the event omits keeps what is displayed; a pot not
/// yet shown is added after the ones shown, with its own population text.
///
/// # Panics
/// If the event's shares for a displayed pot are over a different `population` than the displayed
/// ones (naming the pot): one pot's shares always describe one population.
pub fn merge_equity(rec: &mut Recommendation, eq: &EquitySummary) {
    fn merge(dst: &mut Vec<(Seat, EquityEstimate)>, src: &[(Seat, EquityEstimate)]) {
        for (seat, est) in src {
            match dst.iter_mut().find(|(s, _)| s == seat) {
                Some((_, d)) => {
                    if settled(&est.availability) >= settled(&d.availability) { *d = est.clone(); }
                }
                None => dst.push((*seat, est.clone())),
            }
        }
    }
    merge(&mut rec.equity.hero_combo_vs_each, &eq.hero_combo_vs_each);
    merge(&mut rec.equity.hero_range_vs_each, &eq.hero_range_vs_each);
    for pot in &eq.per_pot_shares {
        match rec.equity.per_pot_shares.iter_mut().find(|p| p.pot_index == pot.pot_index) {
            Some(shown) => {
                assert!(shown.population == pot.population,
                    "merge_equity: pot {} is shown over the population {:?}, the event's shares for it are over {:?}", pot.pot_index, shown.population, pot.population);
                merge(&mut shown.shares, &pot.shares);
            }
            None => rec.equity.per_pot_shares.push(pot.clone()),
        }
    }
}
