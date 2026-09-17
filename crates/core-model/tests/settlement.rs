use core_model::settlement::{check_conservation, layer_pots, refund_uncalled, Settlement};
use core_model::RulesError;
use proto::{Pot, Seat};

fn seats(ids: &[u8]) -> Vec<Seat> { ids.iter().map(|i| Seat(*i)).collect() }
fn pot(amount: u32, eligible: &[u8]) -> Pot { Pot { amount, eligible: seats(eligible) } }

#[test]
fn refund_returns_only_the_uncalled_top() {
    // BTN(5) committed 200 against a next-highest 100: 100 goes back to the stack.
    let mut committed = [50, 100, 0, 0, 0, 200];
    let mut stacks = [0, 0, 0, 0, 0, 0];
    assert_eq!(refund_uncalled(&mut committed, &mut stacks), Some((Seat(5), 100)));
    assert_eq!(committed, [50, 100, 0, 0, 0, 100]);
    assert_eq!(stacks[5], 100);
    assert_eq!(stacks.iter().sum::<u32>() + committed.iter().sum::<u32>(), 350, "a refund moves chips, never creates them");
    // an exactly matched top refunds nothing
    let mut committed = [100, 100, 0, 0, 0, 0];
    let mut stacks = [0; 6];
    assert_eq!(refund_uncalled(&mut committed, &mut stacks), None);
    // two equal tops: the tie must not produce a refund
    let mut committed = [0, 0, 0, 0, 200, 200];
    let mut stacks = [0; 6];
    assert_eq!(refund_uncalled(&mut committed, &mut stacks), None);
    assert_eq!(committed, [0, 0, 0, 0, 200, 200]);
    // nothing committed at all
    let mut committed = [0; 6];
    let mut stacks = [7; 6];
    assert_eq!(refund_uncalled(&mut committed, &mut stacks), None);
    assert_eq!(stacks, [7; 6]);
}

#[test]
fn layer_pots_splits_by_level_and_merges_equal_eligibility() {
    // three all-ins 50 / 100 / 100 after the 200 was refunded down to 100
    assert_eq!(layer_pots(&[50, 100, 0, 0, 0, 100], &[false, false, true, true, true, false]),
               vec![pot(150, &[0, 1, 5]), pot(100, &[1, 5])]);
    // four contributors, nothing uncalled: levels 50, 100, 200
    assert_eq!(layer_pots(&[50, 100, 200, 0, 0, 200], &[false, false, false, true, true, false]),
               vec![pot(200, &[0, 1, 2, 5]), pot(150, &[1, 2, 5]), pot(200, &[2, 5])]);
    // a folded seat's chips are dead money: the level above absorbs them
    assert_eq!(layer_pots(&[100, 2, 0, 0, 0, 100], &[false, true, true, true, true, false]),
               vec![pot(202, &[0, 5])], "one pot of 202, not 6 + 196");
    // adjacent layers with the same eligible set are one pot
    assert_eq!(layer_pots(&[12, 2, 0, 0, 0, 12], &[false, true, true, true, true, false]),
               vec![pot(26, &[0, 5])]);
    // everyone folded to the straddle: blinds and straddle are one pot for the survivor
    assert_eq!(layer_pots(&[1, 2, 4, 0, 0, 0], &[true, true, false, true, true, true]), vec![pot(7, &[2])]);
    assert_eq!(layer_pots(&[0; 6], &[false; 6]), vec![]);
}

#[test]
fn conservation_detects_a_missing_chip() {
    let stacks = [100, 0, 0, 0, 0, 0];
    let live = [0, 50, 0, 0, 0, 0];
    let pots = vec![pot(200, &[0, 1])];
    assert_eq!(check_conservation(&stacks, &live, &pots, 350), Ok(()));
    assert_eq!(check_conservation(&stacks, &live, &pots, 351), Err(RulesError::Conservation { expected: 351, actual: 350 }));
    assert_eq!(Settlement::default(), Settlement { pots: vec![], returned: vec![] });
}

// Review R1: settlement stays infallible, and its numeric domain is fixed by an aggregate
// precondition — the six starting stacks sum to at most `u32::MAX` — that hand admission enforces,
// not settlement. These four tests pin both sides of that boundary: a violation is caught by an
// always-on assertion naming the precondition, and the largest accepted total settles exactly.

#[test]
#[should_panic(expected = "layer total exceeds u32")]
fn a_layer_total_above_u32_is_caught_not_wrapped() {
    // Three matched contributions of 1,500,000,000: each seat value is a valid `u32`, but the
    // single 4,500,000,000-chip layer they form is not.
    let _ = layer_pots(&[1_500_000_000, 1_500_000_000, 1_500_000_000, 0, 0, 0], &[false; 6]);
}

#[test]
#[should_panic(expected = "merged pot exceeds u32")]
fn a_merged_pot_above_u32_is_caught_not_wrapped() {
    // Layers of 1,500,000,000 and 3,000,000,000 each fit `u32` on their own; seat 1 is folded, so
    // both layers are eligible to seats 0 and 2 and merge into 4,500,000,000, which does not.
    let _ = layer_pots(&[2_000_000_000, 500_000_000, 2_000_000_000, 0, 0, 0],
                       &[false, true, false, false, false, false]);
}

#[test]
fn the_largest_accepted_contribution_total_is_u32_max() {
    let pots = layer_pots(&[u32::MAX - 5, 1, 1, 1, 1, 1], &[false; 6]);
    assert_eq!(pots, vec![pot(6, &[0, 1, 2, 3, 4, 5]), pot(u32::MAX - 6, &[0])]);
    let total: u64 = pots.iter().map(|p| u64::from(p.amount)).sum();
    assert_eq!(total, u64::from(u32::MAX), "the boundary case settles exactly u32::MAX chips");
}

#[test]
fn conservation_compares_totals_above_u32_without_wrapping() {
    let stacks = [u32::MAX, u32::MAX, 0, 0, 0, 0];
    let live = [0; 6];
    let pots: Vec<Pot> = vec![];
    // 2 * u32::MAX is 8_589_934_590 exactly in u64; narrowed to u32 it would wrap to 4_294_967_294.
    assert_eq!(check_conservation(&stacks, &live, &pots, 8_589_934_590), Ok(()));
    assert_eq!(check_conservation(&stacks, &live, &pots, 8_589_934_591),
               Err(RulesError::Conservation { expected: 8_589_934_591, actual: 8_589_934_590 }));
}
