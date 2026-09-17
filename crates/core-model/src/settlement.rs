//! Street settlement (spec 4.3): uncalled-bet refunds, side-pot layering and the conservation invariant.
//!
//! **Aggregate precondition (enforced by hand admission, spec 4.3): the six starting stacks sum to at
//! most `u32::MAX`; therefore every pot fits in `u32`. Violations are caught by always-on assertions.**
//! Individually valid `u32` stacks do not by themselves bound their total, so the bound is a named
//! admission-time invariant rather than a property of the types. The admission contract is that
//! `begin_hand` rejects a table whose starting stacks sum above `u32::MAX` with a typed
//! [`RulesError`]; settlement does not re-check it, which is why the functions here are infallible.
//! Chips only move between stacks, live commitments and pots, so the total never grows after
//! admission and every layer and merged pot stays a share of that `u32` total.

use proto::{Pot, Seat};
use crate::error::RulesError;

/// The result of settling a street (spec 4.3): the layered pots and the uncalled portions returned
/// to their owners, in the order they were returned.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Settlement { pub pots: Vec<Pot>, pub returned: Vec<(Seat, u32)> }

/// Returns the uncalled portion of the highest street contribution to its owner (spec 4.3): a transfer
/// from live commitment back to the stack.
///
/// Precondition (enforced by hand admission, spec 4.3): the six starting stacks sum to at most
/// `u32::MAX`; therefore a seat's commitment plus its remaining stack fits in `u32` and the refund,
/// which only moves chips the seat already committed back to it, cannot overflow. Violations are
/// caught by always-on assertions.
pub fn refund_uncalled(committed: &mut [u32; 6], stacks: &mut [u32; 6]) -> Option<(Seat, u32)> {
    let top = (0..6).max_by_key(|i| committed[*i])?;
    let top_amount = committed[top];
    let second = (0..6).filter(|i| *i != top).map(|i| committed[i]).max().unwrap_or(0);
    if top_amount > second {
        let refund = top_amount - second;
        committed[top] = second;
        stacks[top] = stacks[top]
            .checked_add(refund)
            .expect("refunded stack exceeds u32: starting stacks must sum to at most u32::MAX (hand admission invariant)");
        Some((Seat(top as u8), refund))
    } else {
        None
    }
}

/// Layers total contributions into main and side pots by contribution level; folded seats' chips are dead money.
/// Adjacent layers with the same eligible set are merged.
///
/// Precondition (enforced by hand admission, spec 4.3): the six starting stacks sum to at most
/// `u32::MAX`; therefore every pot fits in `u32`. Violations are caught by always-on assertions.
///
/// Six individually valid `u32` contributions can sum past `u32` (three matched 1,500,000,000 stacks
/// suffice), and two layers that each fit can merge into one that does not, so neither narrowing is
/// safe on the types alone — both are guarded here. Layer totals are accumulated in `u64` and
/// narrowed only after the check, never by wraparound.
pub fn layer_pots(contributed: &[u32; 6], folded: &[bool; 6]) -> Vec<Pot> {
    let mut levels: Vec<u32> = contributed.iter().copied().filter(|c| *c > 0).collect();
    levels.sort_unstable();
    levels.dedup();
    let mut pots: Vec<Pot> = Vec::new();
    let mut prev = 0u32;
    for level in levels {
        let wide: u64 = contributed.iter().map(|c| u64::from((*c).min(level) - (*c).min(prev))).sum();
        let amount = u32::try_from(wide).expect("layer total exceeds u32: starting stacks must sum to at most u32::MAX (hand admission invariant)");
        let eligible: Vec<Seat> = (0..6).filter(|i| !folded[*i] && contributed[*i] >= level).map(|i| Seat(i as u8)).collect();
        prev = level;
        if amount == 0 { continue; }
        match pots.last_mut() {
            Some(last) if last.eligible == eligible || eligible.is_empty() => {
                last.amount = last.amount.checked_add(amount).expect("merged pot exceeds u32: starting stacks must sum to at most u32::MAX (hand admission invariant)");
            }
            _ => pots.push(Pot { amount, eligible }),
        }
    }
    pots
}

/// Spec 4.3: `sum(stacks) + live commitments + unawarded pots + rake (0) == sum(stacks_start)`.
///
/// Every term is widened to `u64` before it is summed or compared, so a violation is reported rather
/// than hidden by wraparound.
pub fn check_conservation(stacks: &[u32; 6], live: &[u32; 6], pots: &[Pot], stacks_start_total: u64) -> Result<(), RulesError> {
    let actual = stacks.iter().map(|s| u64::from(*s)).sum::<u64>()
        + live.iter().map(|c| u64::from(*c)).sum::<u64>()
        + pots.iter().map(|p| u64::from(p.amount)).sum::<u64>();
    if actual == stacks_start_total { Ok(()) } else { Err(RulesError::Conservation { expected: stacks_start_total, actual }) }
}
