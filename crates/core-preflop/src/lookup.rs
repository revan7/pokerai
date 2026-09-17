//! Prefix reconstruction and the one preflop lookup entry point, `PreflopStore::query` (spec
//! section 8.3).
//!
//! The rule the whole module exists to keep: **one `PreflopNodeKey` per candidate bundle, built
//! from that candidate's own manifest, never from the live config.** `depth_bb` is the candidate's
//! acquired depth, and `rake_profile`/`straddle` are the candidate's own -- a chart bundle's
//! `rake_profile` is its transcribed text and its `straddle` is `false` even for a straddled hand,
//! because section 8.3's virtual roles are lookup-only. The *live* rake and straddle appear only in
//! the emitted `RakeProfileMapped`/`StraddleMapped` reasons. Building the key from `cfg.rake` or
//! `cfg.straddle` would miss every chart node while still emitting those reasons, which is exactly
//! the failure section 8.3 forbids ("never a guessed node"; "Missing nodes stay missing").

use crate::depth::{asymmetric, bucket, depth_for, prominent_depth, rake_reason, rank_key, start_stack};
use crate::envelope::{BundleInfo, PreflopNode, PreflopNodeKey, PreflopSource, PreflopStep};
use crate::store::{node_key, PreflopStore};
use crate::straddle::{check_straddle, normalized_posts, physical_positions, short_handed_prefix, source_unit, virtual_position};
use core_model::RulesError;
use proto::{Action, ApproxReason, HandConfig, HandPhase, HandState, Position, Seat, Street, UnsupportedReason};
use std::collections::HashMap;

/// One preflop lookup's outcome: the key that was looked up, the physical actor, the node if the
/// selected source has it, the selected bundle, the source unit in chips, the mapping reasons that
/// accumulated on the way and the `Unsupported` reason if there is one.
///
/// Plan-3 owned (spec section 8.1 defines the store, not the answer). It gains
/// `expanded: Option<ExpandedNode>` in Task 9; until then a caller that finds `node: None` and
/// `unsupported: Some(MissingPreflopNode)` has the truth, and no `NodeStrategy` with zero EVs is
/// ever fabricated to fill the gap.
#[derive(Clone, Debug, PartialEq)]
pub struct PreflopAnswer {
    pub key: String,
    pub actor: Option<Seat>,
    pub node: Option<PreflopNode>,
    pub bundle: Option<BundleInfo>,
    pub unit: u32,
    pub reasons: Vec<ApproxReason>,
    pub notes: Vec<String>,
    pub unsupported: Option<UnsupportedReason>,
}

impl PreflopAnswer {
    pub fn empty() -> Self {
        PreflopAnswer {
            key: String::new(),
            actor: None,
            node: None,
            bundle: None,
            unit: 0,
            reasons: vec![],
            notes: vec![],
            unsupported: None,
        }
    }
}

/// The hand as it stood before its `prefix_len`-th recorded action, re-derived from that truncated
/// history: the board is cleared and the phase is set back to preflop betting, so the replay stops
/// at the end of the preflop street instead of demanding board cards for a street this prefix never
/// reached.
///
/// # Panics
/// Panics (in every build profile, per the standing ruling) if `prefix_len` is past the recorded
/// history, if the truncated history contains a postflop action -- naming the offending index --
/// or if the prefix does not replay at all. `core_model::derive` cannot replay a postflop action
/// with no board, and a truncated-but-postflop prefix is not a preflop prefix.
/// `PreflopStore::query` never reaches any of these: it screens the street and reconstructs
/// through [`checked_prefix_state`], which reports the same conditions as typed answers.
pub fn prefix_state(state: &HandState, prefix_len: usize) -> HandState {
    assert!(prefix_len <= state.actions.len(), "prefix_state: prefix {prefix_len} past {} actions", state.actions.len());
    if let Some((i, a)) = state.actions[..prefix_len].iter().enumerate().find(|(_, a)| a.street != Street::Preflop) {
        panic!("prefix_state: action {i} is on {:?}, not preflop", a.street);
    }
    checked_prefix_state(state, prefix_len).unwrap_or_else(|e| panic!("prefix_state: the prefix does not replay: {e}"))
}

/// [`prefix_state`]'s fallible twin, used by [`PreflopStore::query`] so that a `HandState` which
/// does not replay -- an externally deserialized one with zero stacks, a stack vector that does not
/// match `dealt`, or a history that is not legal from the posts -- is a typed answer instead of the
/// panic `core_model::derive` raises by contract (P3.T8 fix round 1, R1). One replay, not two:
/// `simulate` is what `derive` calls.
fn checked_prefix_state(state: &HandState, prefix_len: usize) -> Result<HandState, RulesError> {
    let mut prefix = state.clone();
    prefix.actions.truncate(prefix_len);
    prefix.board.clear();
    prefix.phase = HandPhase::Betting { street: Street::Preflop };
    prefix.derived = core_model::lifecycle::simulate(&prefix)?.derived();
    Ok(prefix)
}

/// A live raise-to in thousandths of a source unit, rounded to the nearest thousandth, or `None`
/// when the source-key domain (`u32` thousandths) cannot represent it. The public hand domain is
/// wider than the key domain -- a 10,000,000-chip raise at a 2-chip big blind is 5,000,000,000
/// thousandths -- so this is a fallible conversion, never a clamp (standing ruling (a)).
///
/// # Panics
/// Panics (in every build profile) on a zero unit.
fn to_bb_x1000(to: u32, unit: u32) -> Option<u32> {
    assert!(unit > 0, "to_bb_x1000: the source unit is at least one chip");
    u32::try_from((u64::from(to) * 1000 + u64::from(unit) / 2) / u64::from(unit)).ok()
}

/// One live chip action as the source step it is looked up as, **without** consulting any source
/// menu: a raise-to is divided by the source unit into `to_bb_x1000`, rounded to the nearest
/// thousandth of a unit, and `PreflopStep::AllIn` carries no amount. This is the menu-unaware view
/// [`observed_history`] exposes; `PreflopStore::query` does not use it, because a key must carry
/// the source's own size (see [`size_matches`] and R3 below).
///
/// # Panics
/// Panics (in every build profile) on a zero unit, or when the raise has no `u32`-thousandths
/// representation -- clamping it would silently look up a different size. `query` reports that
/// condition as a typed `UnsupportedHistory` answer instead (P3.T8 fix round 1, R4).
pub fn to_source_step(action: &Action, unit: u32) -> PreflopStep {
    match action {
        Action::Fold => PreflopStep::Fold,
        Action::Check => PreflopStep::Check,
        Action::Call => PreflopStep::Call,
        Action::Bet { to } | Action::Raise { to } => PreflopStep::Raise {
            to_bb_x1000: to_bb_x1000(*to, unit)
                .unwrap_or_else(|| panic!("to_source_step: {to} chips has no u32-thousandths representation at a {unit}-chip unit")),
        },
        Action::AllIn { .. } => PreflopStep::AllIn,
    }
}

/// Whether a live raise-to of `to` chips is the same size as a source step's `to_bb_x1000`, within
/// section 8.3's `0.5 * chip / unit` tolerance -- the half-chip the chip grid cannot express.
///
/// The comparison is exact and widened, never floating: multiplying
/// `|to / unit - source / 1000| <= 0.5 / unit` through by `1000 * unit` gives
/// `|to * 1000 - source * unit| <= 500`, evaluated in `u64` (P3.T8 fix round 1, R3 -- the earlier
/// floating form rejected `to = 24, source = 2450, unit = 10`, which is exactly half a chip apart
/// and therefore inside an inclusive tolerance). This is the one implementation of the rule; the
/// menu resolution below and Tasks 10/13 all call it.
///
/// # Panics
/// Panics (in every build profile) on a zero unit.
pub fn size_matches(to: u32, source_to_bb_x1000: u32, unit: u32) -> bool {
    assert!(unit > 0, "size_matches: the source unit is at least one chip");
    size_distance(to, source_to_bb_x1000, unit) <= 500
}

/// `|to * 1000 - source * unit|` in `u64`: the exact distance [`size_matches`] thresholds, and the
/// key the menu resolution below picks the closest candidate size by.
fn size_distance(to: u32, source_to_bb_x1000: u32, unit: u32) -> u64 {
    (u64::from(to) * 1000).abs_diff(u64::from(source_to_bb_x1000) * u64::from(unit))
}

/// The closest raise size in `menu` that a live raise-to of `to` chips matches within section 8.3's
/// tolerance, or `None` if the menu offers no such size. Ties resolve to the smaller size, so the
/// choice is deterministic even for two source sizes less than a chip apart. `PreflopStep::AllIn`
/// carries no size and is never a size match.
fn menu_size(menu: &[PreflopStep], to: u32, unit: u32) -> Option<u32> {
    menu.iter()
        .filter_map(|s| match s {
            PreflopStep::Raise { to_bb_x1000 } => Some(*to_bb_x1000),
            _ => None,
        })
        .filter(|&s| size_matches(to, s, unit))
        .min_by_key(|&s| (size_distance(to, s, unit), s))
}

/// The observed preflop prefix as source steps under the (possibly virtual) roles, with each raise
/// converted by [`to_source_step`] alone.
///
/// This is the **menu-unaware** view: it is what a caller holding no candidate source can compute
/// (Tasks 10/13 read branch histories through it). `PreflopStore::query` uses
/// [`resolve_against_source`] instead, which resolves each wager against the selected source's own
/// menu before it enters a key.
///
/// # Panics
/// Inherits [`to_source_step`]'s panics; `query` does not call it.
pub fn observed_history(
    prefix: &HandState,
    roles: &[(Seat, Position)],
    mapped: bool,
    unit: u32,
) -> Vec<(Position, PreflopStep)> {
    prefix
        .actions
        .iter()
        .filter(|a| a.street == proto::Street::Preflop)
        .map(|a| {
            let role = roles.iter().find(|(s, _)| *s == a.seat).expect("dealt seat").1;
            (virtual_position(role, mapped), to_source_step(&a.action, unit))
        })
        .collect()
}

/// A prefix's history resolved against one candidate source (see [`resolve_against_source`]).
struct ResolvedHistory {
    /// The lookup-only short-handed folds followed by the observed steps.
    history: Vec<(Position, PreflopStep)>,
    /// Why a wager could not be resolved onto a size the source offers, if one could not be. The
    /// history is still built -- the off-menu action is preserved at its own size for Tasks 10/13's
    /// translated branch -- but it is not looked up, because the source's tree does not contain
    /// that branch and a rounded size must never select a node the hand did not reach.
    off_menu: Option<String>,
}

/// Resolves each historical wager against the selected source's menu **at the corresponding prefix
/// node**, so a size within section 8.3's half-chip tolerance is looked up as the source's own size
/// and an off-menu size is never rounded into a key (P3.T8 fix round 1, R3).
///
/// The walk rebuilds the key node by node: folds, checks, calls and all-ins map directly, and each
/// raise is compared -- in exact chips, with no thousandth rounding in between -- against the
/// `Raise` sizes the node reached so far actually offers. Once a wager is off-menu (or the source
/// has no node at that prefix, so no menu exists to resolve against) the remaining raises are
/// recorded at their own converted size and no further lookup is attempted.
///
/// `Err` is the one condition that is neither a match nor an off-menu branch: a raise whose size
/// has no representation in the source-key domain at all (R4).
fn resolve_against_source(
    prefix: &HandState,
    roles: &[(Seat, Position)],
    mapped: bool,
    unit: u32,
    short: &[(Position, PreflopStep)],
    source: &dyn PreflopSource,
    depth_bb: u16,
    info: &BundleInfo,
) -> Result<ResolvedHistory, UnsupportedReason> {
    let mut history = short.to_vec();
    let mut off_menu: Option<String> = None;
    for a in prefix.actions.iter().filter(|a| a.street == Street::Preflop) {
        let role = roles.iter().find(|(s, _)| *s == a.seat).expect("dealt seat").1;
        let step = match a.action {
            Action::Fold => PreflopStep::Fold,
            Action::Check => PreflopStep::Check,
            Action::Call => PreflopStep::Call,
            Action::AllIn { .. } => PreflopStep::AllIn,
            Action::Bet { to } | Action::Raise { to } => {
                let node = if off_menu.is_some() {
                    None
                } else {
                    source.lookup(&PreflopNodeKey {
                        depth_bb,
                        rake_profile: info.rake_profile.clone(),
                        straddle: info.straddle,
                        history: history.clone(),
                    })
                };
                match node.as_ref().and_then(|n| menu_size(&n.actions, to, unit)) {
                    Some(source_to) => PreflopStep::Raise { to_bb_x1000: source_to },
                    None => {
                        if off_menu.is_none() {
                            off_menu = Some(match node {
                                Some(_) => format!("seat {}'s raise to {to} chips is not a size the source offers at that node", a.seat.0),
                                None => format!("the source has no node before seat {}'s raise, so its sizes cannot be resolved", a.seat.0),
                            });
                        }
                        PreflopStep::Raise {
                            to_bb_x1000: to_bb_x1000(to, unit).ok_or_else(|| UnsupportedReason::UnsupportedHistory {
                                reason: format!("a raise to {to} chips is not representable in {unit}-chip source units"),
                            })?,
                        }
                    }
                }
            }
        };
        history.push((virtual_position(role, mapped), step));
    }
    Ok(ResolvedHistory { history, off_menu })
}

impl PreflopStore {
    /// The preflop answer for `state`'s decision after its first `prefix_len` actions (spec section
    /// 8.3). Reads only public state: `cfg`, the starting stacks, the recorded actions and each
    /// candidate bundle's manifest. Hero's cards are never consulted, so the answer is identical
    /// whether or not they are on record, and identical before and after any later action --
    /// eligibility, depth and roles are all taken at the prefix.
    pub fn query(&self, cfg: &HandConfig, state: &HandState, prefix_len: usize) -> PreflopAnswer {
        let mut answer = PreflopAnswer::empty();
        // Screened on `state` before `prefix_state`, which cannot re-derive a postflop action with
        // the board cleared (it asserts instead); the reason reported is the same either way.
        if prefix_len > state.actions.len() {
            answer.unsupported = Some(UnsupportedReason::UnsupportedHistory {
                reason: format!("prefix {prefix_len} past {} recorded actions", state.actions.len()),
            });
            return answer;
        }
        if state.actions[..prefix_len].iter().any(|a| a.street != Street::Preflop) {
            answer.unsupported =
                Some(UnsupportedReason::UnsupportedHistory { reason: "preflop query over a postflop prefix".into() });
            return answer;
        }
        if *cfg != state.config {
            answer.unsupported = Some(UnsupportedReason::UnsupportedHistory {
                reason: "config does not match the frozen hand config".into(),
            });
            return answer;
        }
        // The format gate runs on `state`, before reconstruction: it needs only the config, the
        // dealt seats, the button and the starting stacks, all of which the prefix shares, and
        // `core_model::derive` panics by contract on exactly the formats it rejects (R1).
        if let Err(e) = check_straddle(state) {
            answer.unsupported = Some(UnsupportedReason::FormatUnsupported { detail: e.to_string() });
            return answer;
        }
        let mapped = cfg.straddle.is_some();
        let prefix = match checked_prefix_state(state, prefix_len) {
            Ok(prefix) => prefix,
            Err(e) => {
                answer.unsupported =
                    Some(UnsupportedReason::UnsupportedHistory { reason: format!("the prefix does not replay: {e}") });
                return answer;
            }
        };
        let unit = source_unit(cfg);
        let roles = physical_positions(&prefix);
        let Some(actor) = prefix.derived.to_act else {
            answer.unsupported = Some(UnsupportedReason::UnsupportedHistory { reason: "no actor at prefix".into() });
            return answer;
        };
        answer.actor = Some(actor);
        // Eligible = dealt and not folded *at this prefix*, all-in included (section 8.3).
        let eligible: Vec<Seat> =
            prefix.dealt.iter().copied().filter(|s| !prefix.derived.folded[s.0 as usize]).collect();
        let others: Vec<u32> =
            eligible.iter().copied().filter(|&s| s != actor).map(|s| start_stack(&prefix, s)).collect();
        let actual_depth = depth_for(start_stack(&prefix, actor), &others, unit);
        let stacks_bb: Vec<f64> = eligible.iter().map(|&s| start_stack(&prefix, s) as f64 / unit as f64).collect();
        let short = short_handed_prefix(prefix.dealt.len());
        let mut ranked: Vec<&Box<dyn PreflopSource>> = self.bundles.iter().collect();
        ranked.sort_by(|a, b| {
            rank_key(a.bundle_info(), actual_depth, &cfg.rake, unit)
                .partial_cmp(&rank_key(b.bundle_info(), actual_depth, &cfg.rake, unit))
                .expect("every rank component is finite or infinite, never NaN")
                .then_with(|| a.bundle_info().bundle_id.cmp(&b.bundle_info().bundle_id))
        });
        let Some(candidate) = ranked.first() else {
            answer.unsupported = Some(UnsupportedReason::MissingPreflopNode { key: "no bundle".into() });
            return answer;
        };
        let info = candidate.bundle_info();
        let Some(used) = bucket(actual_depth, &info.depths) else {
            answer.unsupported = Some(UnsupportedReason::MissingPreflopNode { key: "no acquired depth".into() });
            return answer;
        };
        answer.bundle = Some(info.clone());
        answer.unit = unit;
        if actual_depth != used as f64 {
            answer.reasons.push(ApproxReason::DepthBucket {
                seat: actor,
                actual_bb: actual_depth as f32,
                used_bb: used,
                prominent: prominent_depth(actual_depth, used),
            });
        }
        let (label, prominent) = asymmetric(&stacks_bb, used);
        if label {
            answer.reasons.push(ApproxReason::AsymmetricStacks {
                stacks_bb: stacks_bb.iter().map(|&s| s as f32).collect(),
                prominent,
            });
        }
        if let Some(reason) = rake_reason(&cfg.rake, info, unit) {
            answer.reasons.push(reason);
        }
        if mapped {
            answer.reasons.push(ApproxReason::StraddleMapped { posts: normalized_posts(cfg.sb_chips, cfg.bb_chips, unit) });
        }
        if !short.is_empty() {
            answer.reasons.push(ApproxReason::ShortHandedMapped { dealt: prefix.dealt.len() as u8 });
        }
        // Every wager is resolved against the selected source's own menus before it can enter the
        // key (R3); the mapping reasons above are already recorded, because reasons accumulate even
        // when the history itself turns out not to be answerable.
        let resolved = match resolve_against_source(&prefix, &roles, mapped, unit, &short, candidate.as_ref(), used, info) {
            Ok(resolved) => resolved,
            Err(reason) => {
                answer.unsupported = Some(reason);
                return answer;
            }
        };
        // The key is built from the CANDIDATE, not from the live config.
        let key = PreflopNodeKey {
            depth_bb: used,
            rake_profile: info.rake_profile.clone(),
            straddle: info.straddle,
            history: resolved.history,
        };
        answer.key = node_key(&key);
        // An off-menu wager puts the hand on a branch the source's tree does not contain, so there
        // is nothing to look up: the action is preserved at its own size for Tasks 10/13 to
        // translate, and the node is reported missing rather than resolved to a neighbouring size.
        //
        // The reported key carries the off-menu marker, because the only size a `PreflopStep` can
        // carry is a thousandth of a unit and that conversion can land on a size the source does
        // offer (7501 chips at a 3000-chip unit rounds to the source's own 2500). Marking it keeps
        // the answer's key from ever being equal to a real node key -- the same way the brief's own
        // `"no bundle"` and `"no acquired depth"` labels are not node keys.
        if let Some(cause) = resolved.off_menu {
            answer.key = format!("{} [off-menu: {cause}]", answer.key);
            answer.notes.push(format!("off-menu wager kept for translation: {cause}"));
            answer.unsupported = Some(UnsupportedReason::MissingPreflopNode { key: answer.key.clone() });
            return answer;
        }
        // A miss in the selected source is final: a lower-ranked bundle is never searched to hide
        // it (section 8.3, "Missing nodes stay missing").
        match candidate.lookup(&key) {
            Some(node) => answer.node = Some(node),
            None => answer.unsupported = Some(UnsupportedReason::MissingPreflopNode { key: answer.key.clone() }),
        }
        answer
    }
}

/// One replay invocation's memo of preflop mappings (spec section 8.3 via the brief's Step 4:
/// "Cache the selected mapping by prefix and branch-translated history within a replay
/// invocation"), added in P3.T8 fix round 1, R5.
///
/// The memo is **owned by the invocation, never by the store**: `PreflopStore` stays stateless and
/// shareable (`&self`, `Send + Sync`), and one of these lives for exactly as long as one replay run
/// -- Task 13's driver holds it, and [`PreflopInvocation::reset`] (or dropping it) ends the run.
/// Nothing here is persisted.
///
/// An entry is keyed by everything a mapping depends on within a run: the hand identity and
/// revision, the config revision, the dealt seats and their starting stacks, the prefix length, and
/// the prefix's own chip history -- which is what makes a *branch-translated* history a distinct
/// entry, since a driver exploring a translated branch passes a state whose recorded actions are
/// that branch. A caller must not reuse one invocation across two different stores.
#[derive(Debug, Default)]
pub struct PreflopInvocation {
    entries: HashMap<MappingKey, PreflopAnswer>,
    lookups: u64,
    hits: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MappingKey {
    hand_id: u64,
    hand_revision: u32,
    config_revision: u32,
    prefix_len: usize,
    dealt: Vec<Seat>,
    stacks_start: Vec<u32>,
    history: Vec<(Seat, Action)>,
}

impl PreflopInvocation {
    pub fn new() -> Self {
        Self::default()
    }

    /// [`PreflopStore::query`]'s answer for this prefix, computed once per distinct mapping within
    /// this invocation. The returned answer is identical to calling `store.query` directly.
    pub fn answer(
        &mut self,
        store: &PreflopStore,
        cfg: &HandConfig,
        state: &HandState,
        prefix_len: usize,
    ) -> PreflopAnswer {
        self.lookups += 1;
        let key = MappingKey {
            hand_id: state.hand_id,
            hand_revision: state.hand_revision,
            config_revision: cfg.config_revision,
            prefix_len,
            dealt: state.dealt.clone(),
            stacks_start: state.stacks_start.clone(),
            history: state
                .actions
                .iter()
                .take(prefix_len.min(state.actions.len()))
                .map(|a| (a.seat, a.action))
                .collect(),
        };
        if let Some(cached) = self.entries.get(&key) {
            self.hits += 1;
            return cached.clone();
        }
        let answer = store.query(cfg, state, prefix_len);
        self.entries.insert(key, answer.clone());
        answer
    }

    /// Calls made to [`PreflopInvocation::answer`] in this run.
    pub fn lookups(&self) -> u64 {
        self.lookups
    }

    /// Calls that were served from the memo.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Distinct mappings held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Ends the run: no mapping and no counter carries into the next one.
    pub fn reset(&mut self) {
        self.entries.clear();
        self.lookups = 0;
        self.hits = 0;
    }
}
