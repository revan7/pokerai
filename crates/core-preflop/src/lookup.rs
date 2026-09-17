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
use proto::{Action, ApproxReason, HandConfig, HandState, Position, Seat, Street, UnsupportedReason};

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
/// history, or if the truncated history contains a postflop action -- naming the offending index.
/// `core_model::derive` cannot replay a postflop action with no board, and a truncated-but-postflop
/// prefix is not a preflop prefix at all. `PreflopStore::query` screens for it and returns
/// `UnsupportedHistory` rather than reaching this assertion.
pub fn prefix_state(state: &HandState, prefix_len: usize) -> HandState {
    assert!(prefix_len <= state.actions.len(), "prefix_state: prefix {prefix_len} past {} actions", state.actions.len());
    let mut prefix = state.clone();
    prefix.actions.truncate(prefix_len);
    if let Some((i, a)) = prefix.actions.iter().enumerate().find(|(_, a)| a.street != Street::Preflop) {
        panic!("prefix_state: action {i} is on {:?}, not preflop", a.street);
    }
    prefix.board.clear();
    prefix.phase = proto::HandPhase::Betting { street: proto::Street::Preflop };
    prefix.derived = core_model::derive(&prefix);
    prefix
}

/// One live chip action as the source step it is looked up as. A raise-to is divided by the source
/// unit into `to_bb_x1000`, rounded to the nearest thousandth of a unit; `PreflopStep::AllIn`
/// carries no amount, so an all-in's size lives in the hand state alone.
///
/// The size *match* against a node's resolved menu is [`size_matches`], which needs the candidate
/// node's own sizes and therefore belongs where the menu is known (Tasks 10 and 13). Nothing here
/// rounds an off-menu size into the key.
///
/// # Panics
/// Panics (in every build profile) on a zero unit, or on a raise so large in units of a thousandth
/// that `to_bb_x1000` would not fit `u32` -- the key type cannot represent it, and clamping it
/// would silently look up a different size (standing ruling (a)).
pub fn to_source_step(action: &Action, unit: u32) -> PreflopStep {
    assert!(unit > 0, "to_source_step: the source unit is at least one chip");
    let x1000 = |to: u32| -> u32 {
        let wide = (u64::from(to) * 1000 + u64::from(unit) / 2) / u64::from(unit);
        u32::try_from(wide).unwrap_or_else(|_| panic!("to_source_step: {to} chips is {wide} thousandths of a {unit}-chip unit, above u32"))
    };
    match action {
        Action::Fold => PreflopStep::Fold,
        Action::Check => PreflopStep::Check,
        Action::Call => PreflopStep::Call,
        Action::Bet { to } | Action::Raise { to } => PreflopStep::Raise { to_bb_x1000: x1000(*to) },
        Action::AllIn { .. } => PreflopStep::AllIn,
    }
}

/// Whether a live raise-to of `to` chips is the same size as a source step's `to_bb_x1000`, within
/// section 8.3's `0.5 * chip / unit` tolerance -- the half-chip the chip grid cannot express. An
/// off-menu size fails this and is handled as a translated branch (Tasks 10 and 13), never rounded
/// into a key.
///
/// # Panics
/// Panics (in every build profile) on a zero unit.
pub fn size_matches(to: u32, source_to_bb_x1000: u32, unit: u32) -> bool {
    assert!(unit > 0, "size_matches: the source unit is at least one chip");
    let live = to as f64 / unit as f64;
    let source = source_to_bb_x1000 as f64 / 1000.0;
    (live - source).abs() <= 0.5 / unit as f64
}

/// The observed preflop prefix as source steps under the (possibly virtual) roles.
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
        let prefix = prefix_state(state, prefix_len);
        let mapped = cfg.straddle.is_some();
        if let Err(e) = check_straddle(&prefix) {
            answer.unsupported = Some(UnsupportedReason::FormatUnsupported { detail: e.to_string() });
            return answer;
        }
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
        let observed = observed_history(&prefix, &roles, mapped, unit);
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
        // The key is built from the CANDIDATE, not from the live config.
        let key = PreflopNodeKey {
            depth_bb: used,
            rake_profile: info.rake_profile.clone(),
            straddle: info.straddle,
            history: short.iter().cloned().chain(observed.iter().cloned()).collect(),
        };
        answer.key = node_key(&key);
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
        // A miss in the selected source is final: a lower-ranked bundle is never searched to hide
        // it (section 8.3, "Missing nodes stay missing").
        match candidate.lookup(&key) {
            Some(node) => answer.node = Some(node),
            None => answer.unsupported = Some(UnsupportedReason::MissingPreflopNode { key: answer.key.clone() }),
        }
        answer
    }
}
