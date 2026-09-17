//! Virtual roles, the source unit, physical positions, short-handed mapping and the phase-1
//! straddle format gate (spec section 8.3).
//!
//! All four mappings are **lookup-only**: they decide which source node a live decision is looked
//! up as, and never change the hand. The straddle's physical posts stay exactly as `core-model`
//! recorded them, the missing seats of a short-handed table are never given actions or ranges, and
//! nothing here writes to `HandState`.

use crate::depth::start_stack;
use crate::envelope::PreflopStep;
use core_model::RulesError;
use proto::{HandConfig, HandState, Position, Seat};

/// The unit every source size is expressed in: the big blind, or the straddle when one is posted
/// (spec section 8.3 -- with a straddle the source's "1 bb" is the straddle).
pub fn source_unit(cfg: &HandConfig) -> u32 {
    cfg.straddle.as_ref().map(|s| s.amount_chips).unwrap_or(cfg.bb_chips)
}

/// Dealt seats clockwise from the button: indices 0, 1, 2 are BTN, SB, BB and the remaining names
/// are the last `n - 3` of UTG, HJ, CO (spec section 4.3), which is the same assignment
/// `core_model::positions` makes from the other end of the ring.
///
/// # Panics
/// Panics (in every build profile, per the standing ruling) on a seat id or dealt count outside the
/// supported range, naming the offending value; `validate_table` is what normally guarantees both.
pub fn physical_positions(s: &HandState) -> Vec<(Seat, Position)> {
    use Position::*;
    assert!(s.button.0 < 6, "physical_positions: button seat {} is outside 0..6", s.button.0);
    for seat in &s.dealt {
        assert!(seat.0 < 6, "physical_positions: dealt seat {} is outside 0..6", seat.0);
    }
    let mut seats = s.dealt.clone();
    seats.sort_by_key(|seat| (seat.0 + 6 - s.button.0) % 6);
    let n = seats.len();
    assert!((3..=6).contains(&n), "physical_positions: {n} dealt seats; 3 to 6 are supported");
    seats
        .into_iter()
        .enumerate()
        .map(|(i, seat)| (seat, match i { 0 => Btn, 1 => Sb, 2 => Bb, _ => [Utg, Hj, Co][6 - n + i - 3] }))
        .collect()
}

/// The lookup-only folds a short-handed table is mapped through: UTG, HJ, CO folded at three dealt
/// seats; UTG, HJ at four; UTG at five; empty at six (spec section 8.3). These are never appended
/// to `HandState.actions` and no range is ever created for them.
///
/// # Panics
/// Panics (in every build profile) outside 3..=6 dealt seats.
pub fn short_handed_prefix(n: usize) -> Vec<(Position, PreflopStep)> {
    assert!((3..=6).contains(&n), "short_handed_prefix: {n} dealt seats; 3 to 6 are supported");
    [Position::Utg, Position::Hj, Position::Co].into_iter().take(6 - n).map(|p| (p, PreflopStep::Fold)).collect()
}

/// The role a physical position is looked up as behind a posted UTG straddle (spec section 8.3's
/// virtual-role table): the straddler is the virtual BB and every other seat shifts one step back.
/// With `mapped == false` the physical position is its own role.
pub fn virtual_position(p: Position, mapped: bool) -> Position {
    use Position::*;
    if !mapped {
        return p;
    }
    match p {
        Hj => Utg,
        Co => Hj,
        Btn => Co,
        Sb => Btn,
        Bb => Sb,
        Utg => Bb,
    }
}

/// The posts reported under `ApproxReason::StraddleMapped`, normalized to the straddle: `(sb/S,
/// bb/S, 1)` (spec section 8.3). The physical posts themselves are untouched.
///
/// # Panics
/// Panics (in every build profile) on a straddle of zero chips, which would report `[inf, inf, 1]`
/// -- a value the wire cannot carry back (the same precondition `core_model::straddle_posts` has).
pub fn normalized_posts(sb: u32, bb: u32, s: u32) -> [f32; 3] {
    assert!(s > 0, "normalized_posts: the straddle is at least one chip");
    [sb as f32 / s as f32, bb as f32 / s as f32, 1.0]
}

/// Section 8.3's phase-1 straddle format gate, applied before any lookup. The dealt-count and
/// `S >= 2 * bb` rules live in Plan 1's `validate_table`; this reuses them rather than restating
/// them, and adds the one condition `validate_table` does not check: the straddler's starting stack
/// must cover the full post.
///
/// `HandConfig.straddle` is `Option<UtgStraddle>`, so a re-straddle cannot be represented at all --
/// it fails `HandConfig` deserialization, before any query. This gate exists because an externally
/// deserialized state can still carry a short post or the wrong dealt count.
pub fn check_straddle(state: &HandState) -> Result<(), RulesError> {
    let cfg = &state.config;
    core_model::validate_table(cfg, state.button, &state.dealt)?;
    // The one shape `validate_table` does not cover and `start_stack` requires. An externally
    // deserialized state is exactly what this gate is for, so a mismatch is a typed error here
    // rather than the always-on panic `start_stack` would raise below.
    if state.stacks_start.len() != state.dealt.len() {
        return Err(RulesError::InvalidConfig {
            reason: format!("{} starting stacks for {} dealt seats", state.stacks_start.len(), state.dealt.len()),
        });
    }
    let Some(s) = &cfg.straddle else { return Ok(()) };
    let roles = physical_positions(state);
    let straddler = roles
        .iter()
        .find(|(_, p)| *p == Position::Utg)
        .map(|(seat, _)| *seat)
        .ok_or(RulesError::FormatUnsupported { detail: "no UTG seat to straddle".into() })?;
    if start_stack(state, straddler) < s.amount_chips {
        return Err(RulesError::FormatUnsupported {
            detail: "straddler's starting stack does not cover the straddle".into(),
        });
    }
    Ok(())
}
