use proto::{Pot, Seat};
use crate::error::RulesError;

/// The result of settling a street (spec 4.3): the layered pots and the uncalled portions returned
/// to their owners, in the order they were returned.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Settlement { pub pots: Vec<Pot>, pub returned: Vec<(Seat, u32)> }

/// Returns the uncalled portion of the highest street contribution to its owner (spec 4.3): a transfer
/// from live commitment back to the stack.
///
/// Infallible, so the caller's contract is enforced with an always-on check: the refund is chips the
/// seat already owned, so returning them cannot overflow its stack.
pub fn refund_uncalled(committed: &mut [u32; 6], stacks: &mut [u32; 6]) -> Option<(Seat, u32)> {
    let top = (0..6).max_by_key(|i| committed[*i])?;
    let top_amount = committed[top];
    let second = (0..6).filter(|i| *i != top).map(|i| committed[i]).max().unwrap_or(0);
    if top_amount > second {
        let refund = top_amount - second;
        committed[top] = second;
        stacks[top] = stacks[top]
            .checked_add(refund)
            .expect("a refund returns chips the seat already committed, so its stack cannot overflow");
        Some((Seat(top as u8), refund))
    } else {
        None
    }
}

/// Layers total contributions into main and side pots by contribution level; folded seats' chips are dead money.
/// Adjacent layers with the same eligible set are merged.
///
/// Infallible, so the caller's contract is enforced with always-on checks: contributions are shares of the
/// starting stacks, so every layer total fits in `u32`. Layer totals are accumulated in `u64` first and
/// narrowed only after that check, never by wraparound.
pub fn layer_pots(contributed: &[u32; 6], folded: &[bool; 6]) -> Vec<Pot> {
    let mut levels: Vec<u32> = contributed.iter().copied().filter(|c| *c > 0).collect();
    levels.sort_unstable();
    levels.dedup();
    let mut pots: Vec<Pot> = Vec::new();
    let mut prev = 0u32;
    for level in levels {
        let wide: u64 = contributed.iter().map(|c| u64::from((*c).min(level) - (*c).min(prev))).sum();
        let amount = u32::try_from(wide).expect("a pot is a share of the starting stacks and fits in u32");
        let eligible: Vec<Seat> = (0..6).filter(|i| !folded[*i] && contributed[*i] >= level).map(|i| Seat(i as u8)).collect();
        prev = level;
        if amount == 0 { continue; }
        match pots.last_mut() {
            Some(last) if last.eligible == eligible || eligible.is_empty() => {
                last.amount = last.amount.checked_add(amount).expect("a pot is a share of the starting stacks and fits in u32");
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
