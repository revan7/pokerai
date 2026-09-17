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
