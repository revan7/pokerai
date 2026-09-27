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
use crate::ev::{expand_node, ExpandedNode};
use crate::store::{node_key, PreflopStore};
use crate::straddle::{check_straddle, normalized_posts, physical_positions, short_handed_prefix, source_unit, virtual_position};
use core_model::RulesError;
use proto::{Action, ApproxReason, HandConfig, HandPhase, HandState, Position, Seat, Street, UnsupportedReason};
use std::collections::HashMap;

/// One preflop lookup's outcome: the key that was looked up, the physical actor, the node if the
/// selected source has it, the selected bundle, the source unit in chips, the mapping reasons that
/// accumulated on the way and the `Unsupported` reason if there is one.
///
/// Plan-3 owned (spec section 8.1 defines the store, not the answer). `expanded` is `Some` exactly
/// when `node` is: the same source node, expanded from 169 classes to 1326 combos and from source
/// SB to chips (P3.T9, [`crate::ev::expand_node`]). A caller that finds `node: None` and
/// `unsupported: Some(MissingPreflopNode)` has the truth, and no `NodeStrategy` with zero EVs is
/// ever fabricated to fill the gap.
#[derive(Clone, Debug, PartialEq)]
pub struct PreflopAnswer {
    pub key: String,
    pub actor: Option<Seat>,
    pub node: Option<PreflopNode>,
    pub expanded: Option<ExpandedNode>,
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
            expanded: None,
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
    menu_step_index(menu, &Action::Raise { to }, unit).map(|i| match menu[i] {
        PreflopStep::Raise { to_bb_x1000 } => to_bb_x1000,
        ref other => unreachable!("menu_step_index resolves a raise onto a Raise step, not {other:?}"),
    })
}

/// The position in one source node's `menu` of the step a live chip `action` is (spec section
/// 8.3's size rule; P3.T13): `Fold`, `Check` and `Call` are their like-named step; a `Bet` or
/// `Raise` is the closest `Raise` size within [`size_matches`]'s half-chip tolerance, ties to the
/// smaller size -- the same resolution the key walk below applies to every historical wager, so an
/// action on the menu here is the step a key built from it carries; an `AllIn` is the `AllIn` step
/// (the source's all-in, which expands to the actor's own maximum). `None` when the node offers no
/// such step: an off-menu wager, which section 8.4 translates, or a non-wager this node does not
/// list, for which the node has no likelihood at all.
///
/// # Panics
/// Panics (in every build profile) on a zero unit, through [`size_matches`].
pub fn menu_step_index(menu: &[PreflopStep], action: &Action, unit: u32) -> Option<usize> {
    let like = |step: PreflopStep| menu.iter().position(|s| *s == step);
    match action {
        Action::Fold => like(PreflopStep::Fold),
        Action::Check => like(PreflopStep::Check),
        Action::Call => like(PreflopStep::Call),
        Action::AllIn { .. } => like(PreflopStep::AllIn),
        Action::Bet { to } | Action::Raise { to } => menu
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s {
                PreflopStep::Raise { to_bb_x1000 } => Some((i, *to_bb_x1000)),
                _ => None,
            })
            .filter(|&(_, s)| size_matches(*to, s, unit))
            .min_by_key(|&(_, s)| (size_distance(*to, s, unit), s))
            .map(|(i, _)| i),
    }
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
/// `steps` is the chip history the key is built from: the observed preflop prefix for
/// [`PreflopStore::query`], or a replay branch's translated history for
/// [`PreflopStore::query_history`] (P3.T13). A translated wager is a menu action the replay took
/// from an expanded node -- the source size rounded to the chip -- so it resolves back onto that
/// source size here, which a thousandth conversion of the rounded chips would not (7.5 chips round
/// to 8, and 8 chips convert to 2667, not the source's 2500).
///
/// `Err` is the one condition that is neither a match nor an off-menu branch: a raise whose size
/// has no representation in the source-key domain at all (R4).
fn resolve_against_source(
    steps: &[(Seat, Action)],
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
    for &(seat, action) in steps {
        let role = roles.iter().find(|(s, _)| *s == seat).expect("dealt seat").1;
        let step = match action {
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
                                Some(_) => format!("seat {}'s raise to {to} chips is not a size the source offers at that node", seat.0),
                                None => format!("the source has no node before seat {}'s raise, so its sizes cannot be resolved", seat.0),
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
        self.answer_for(cfg, state, prefix_len, None)
    }

    /// [`PreflopStore::query`] with one substitution (P3.T13, spec section 8.4's history branches):
    /// the node history is `history` -- a replay branch's translated history, each translated wager
    /// replaced by its mapped menu action -- instead of the observed prefix's own actions.
    ///
    /// Everything else is the observed prefix's, exactly as `query` computes it: the actor, the
    /// eligible seats, the depth bucket, the rake profile, the physical and virtual roles, the
    /// short-handed folds, the selected bundle, the source unit, the actor's actual chip maximum
    /// for `AllIn`, and every mapping reason (section 8.3 is hindsight-free, and actual money is
    /// never rewritten). A branch that translated villain's raise to menu size A therefore looks up
    /// the next actor's node under A. Each wager in `history` is resolved against the selected
    /// source's own menus exactly as the observed prefix's are (see `resolve_against_source`), so
    /// a chip-rounded menu action finds the source size it came from.
    ///
    /// `history` must follow the observed prefix actor by actor (one entry per preflop action of the
    /// prefix, same seats in the same order): a translated history differs from the observed one only
    /// in its wager sizes. Anything else is a typed `UnsupportedHistory` answer, never a lookup of a
    /// different decision.
    pub fn query_history(&self, cfg: &HandConfig, state: &HandState, prefix_len: usize, history: &[(Seat, Action)]) -> PreflopAnswer {
        self.answer_for(cfg, state, prefix_len, Some(history))
    }

    /// The shared body of [`PreflopStore::query`] (`history == None`: the observed prefix) and
    /// [`PreflopStore::query_history`] (`Some`: a branch's translated history).
    fn answer_for(&self, cfg: &HandConfig, state: &HandState, prefix_len: usize, history: Option<&[(Seat, Action)]>) -> PreflopAnswer {
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
        // The chip history the key is built from: the observed prefix, or (P3.T13) a branch's
        // translated history, which must follow the observed prefix actor by actor.
        let observed: Vec<(Seat, Action)> =
            prefix.actions.iter().filter(|a| a.street == Street::Preflop).map(|a| (a.seat, a.action)).collect();
        let steps: &[(Seat, Action)] = match history {
            None => &observed,
            Some(translated) => {
                let follows = translated.len() == observed.len()
                    && translated.iter().zip(&observed).all(|((seat, _), (actual, _))| seat == actual);
                if !follows {
                    answer.unsupported = Some(UnsupportedReason::UnsupportedHistory {
                        reason: format!(
                            "a translated history by seats {:?} does not follow the observed prefix's actors {:?}",
                            translated.iter().map(|(s, _)| s.0).collect::<Vec<_>>(),
                            observed.iter().map(|(s, _)| s.0).collect::<Vec<_>>()
                        ),
                    });
                    return answer;
                }
                translated
            }
        };
        // Every wager is resolved against the selected source's own menus before it can enter the
        // key (R3); the mapping reasons above are already recorded, because reasons accumulate even
        // when the history itself turns out not to be answerable.
        let resolved = match resolve_against_source(steps, &roles, mapped, unit, &short, candidate.as_ref(), used, info) {
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
            Some(node) => {
                // P3.T13: the node found must be this actor's decision. The source's turn order
                // follows the key's history, and an all-in in that history (a live raise translated
                // to the source's all-in, or a live all-in call the source records as a plain call)
                // can make it name a different seat than the one acting at this prefix. That node
                // holds no likelihood for this actor, so the actor has no node here -- reported
                // missing, with a marker that keeps the key from equalling a real node key.
                let role = roles.iter().find(|(s, _)| *s == actor).expect("the actor is a dealt seat").1;
                let acting_as = virtual_position(role, mapped);
                if node.actor != acting_as {
                    answer.key = format!(
                        "{} [actor mismatch: the node there is {:?}'s decision, seat {} acts as {acting_as:?}]",
                        answer.key, node.actor, actor.0
                    );
                    answer.notes.push(format!("the source's node at this history belongs to {:?}", node.actor));
                    answer.unsupported = Some(UnsupportedReason::MissingPreflopNode { key: answer.key.clone() });
                    return answer;
                }
                // The actor's actual chip maximum (P3.T9): committed-this-street plus what
                // remains, never the source's own declared depth -- a live stack shallower or
                // deeper than the source's acquired depth still expands `AllIn` to what this
                // actor can actually put in.
                let seat_i = actor.0 as usize;
                let actor_max_to = prefix.derived.committed_this_street[seat_i]
                    .checked_add(prefix.derived.stacks_remaining[seat_i])
                    .expect("a seat never holds more than its starting stack");
                answer.expanded = Some(expand_node(&node, info, actor, unit, actor_max_to));
                answer.node = Some(node);
            }
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
/// revision, the config revision, the dealt seats and their starting stacks, the prefix length, the
/// prefix's own chip history and -- for [`PreflopInvocation::answer_history`] (P3.T13) -- the
/// branch-translated history the node was looked up under. That last component is what makes two
/// history branches at the same observed prefix distinct entries; no final-hand state (later folds,
/// the board) is part of any key. A caller must not reuse one invocation across two different
/// stores.
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
    /// `None` for [`PreflopStore::query`] (the observed prefix is the node history); `Some` for
    /// [`PreflopStore::query_history`], holding the translated history.
    translated: Option<Vec<(Seat, Action)>>,
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
        self.memoized(store, cfg, state, prefix_len, None)
    }

    /// [`PreflopStore::query_history`]'s answer for this observed prefix index and translated
    /// history, computed once per distinct pair within this invocation (P3.T13). The returned
    /// answer is identical to calling `store.query_history` directly.
    pub fn answer_history(
        &mut self,
        store: &PreflopStore,
        cfg: &HandConfig,
        state: &HandState,
        prefix_len: usize,
        history: &[(Seat, Action)],
    ) -> PreflopAnswer {
        self.memoized(store, cfg, state, prefix_len, Some(history))
    }

    fn memoized(
        &mut self,
        store: &PreflopStore,
        cfg: &HandConfig,
        state: &HandState,
        prefix_len: usize,
        translated: Option<&[(Seat, Action)]>,
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
            translated: translated.map(<[(Seat, Action)]>::to_vec),
        };
        if let Some(cached) = self.entries.get(&key) {
            self.hits += 1;
            return cached.clone();
        }
        let answer = match translated {
            None => store.query(cfg, state, prefix_len),
            Some(history) => store.query_history(cfg, state, prefix_len, history),
        };
        self.entries.insert(key, answer.clone());
        answer
    }

    /// Calls made to [`PreflopInvocation::answer`] and [`PreflopInvocation::answer_history`] in
    /// this run.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{EvReference, SourceKind};
    use core_model::state::BeginHand;

    /// A test double whose every lookup returns a node that is the UTG position's decision --
    /// the shape a source's turn order produces when its history's all-ins differ from the live
    /// hand's. The committed synthetic fixture holds no such history.
    struct UtgEverywhere {
        info: BundleInfo,
    }

    impl PreflopSource for UtgEverywhere {
        fn bundle_info(&self) -> &BundleInfo {
            &self.info
        }
        fn lookup(&self, _key: &PreflopNodeKey) -> Option<PreflopNode> {
            Some(PreflopNode {
                actor: Position::Utg,
                actions: vec![PreflopStep::Fold, PreflopStep::Call],
                probs: vec![vec![1.0, 0.0]; 169],
                ev_source_sb: None,
                unreachable: [false; 169],
                committed_by_actor_sb: 0.0,
                fold_wide_verified: false,
            })
        }
        fn nodes_have_no_ev(&self) -> bool {
            true
        }
    }

    /// P3.T13: a node found at the key's history that is another position's decision is not this
    /// actor's node -- `MissingPreflopNode` with a marked key, on the observed and the translated
    /// path alike, and never an expanded node whose likelihoods belong to someone else.
    #[test]
    fn a_node_that_is_another_positions_decision_is_missing_for_this_actor() {
        let info = BundleInfo {
            bundle_id: "utg_everywhere".into(),
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
        };
        let store = PreflopStore::from_sources(vec![Box::new(UtgEverywhere { info })]);
        let cfg = HandConfig { config_revision: 1, sb_chips: 1, bb_chips: 2, straddle: None, rake: proto::Rake::TimeCharge, chip_label: "$1".into() };
        let root = core_model::begin_hand(
            &cfg,
            BeginHand { hand_id: 1, button: Seat(5), hero: Seat(0), dealt: (0..6).map(Seat).collect(), stacks_start: vec![200; 6], hero_cards: None },
        )
        .expect("a six-max table");
        // Control: UTG (seat 2) acts at the root, and the node is UTG's.
        let at_root = store.query(&cfg, &root, 0);
        assert!(at_root.node.is_some() && at_root.expanded.is_some(), "{at_root:?}");
        // After UTG folds the HJ seat acts, but the node the double returns is still UTG's.
        let folded = core_model::apply_action(&root, Action::Fold).expect("UTG folds");
        let answer = store.query(&cfg, &folded, 1);
        assert!(answer.node.is_none() && answer.expanded.is_none(), "{answer:?}");
        assert!(answer.key.contains("actor mismatch") && answer.key.contains("Utg") && answer.key.contains("Hj"), "{}", answer.key);
        assert_eq!(answer.unsupported, Some(UnsupportedReason::MissingPreflopNode { key: answer.key.clone() }));
        assert_eq!(answer.actor, Some(Seat(3)));
        assert_eq!(store.query_history(&cfg, &folded, 1, &[(Seat(2), Action::Fold)]), answer, "the translated path agrees");
    }
}
