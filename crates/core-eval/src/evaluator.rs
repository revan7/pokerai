use holdem_hand_evaluator::Hand;
use proto::Card;

/// Hand-rank backend: ranks are "higher is stronger"; 5 to 7 cards in total.
pub trait Evaluator: Send + Sync {
    type Partial: Clone + Send + Sync;
    /// Pre-combines 0 to 5 cards (a board) for repeated evaluation.
    fn partial(&self, cards: &[Card]) -> Self::Partial;
    /// Rank of `partial` plus `extra` (5 to 7 cards in total).
    fn rank_with(&self, partial: &Self::Partial, extra: &[Card]) -> u16;
    fn rank(&self, cards: &[Card]) -> u16 { self.rank_with(&self.partial(&[]), cards) }
}

/// b-inary/holdem-hand-evaluator (MIT); `rs_poker` is the named contingency only if V1 fails.
#[derive(Clone, Copy, Debug, Default)]
pub struct BinaryEvaluator;

/// Opaque, validated pre-combination of 0 to 5 cards.
///
/// The pinned backend (`holdem_hand_evaluator::Hand`) builds and evaluates hands with
/// `get_unchecked`: an out-of-range card id, a duplicate card, or a card count outside 5..=7
/// is undefined behaviour reachable from safe code, and `proto::Card(pub u8)` lets a caller
/// construct an invalid id (e.g. `Card(255)`) trivially. `BinaryEvaluator::partial` is the only
/// way to build a `PartialHand`: its fields are private, so a caller cannot hand the backend an
/// arbitrary raw `Hand` that skipped these checks.
#[derive(Clone, Copy, Debug)]
pub struct PartialHand {
    hand: Hand,
    mask: u64,
    count: u8,
}

impl Evaluator for BinaryEvaluator {
    type Partial = PartialHand;

    fn partial(&self, cards: &[Card]) -> PartialHand {
        assert!(cards.len() <= 5, "partial: {} cards exceeds the 5-card board limit", cards.len());
        // Validate every id and check for duplicates (a mask bit test plus one count check per
        // card, no allocation) before touching the backend at all.
        let mut mask: u64 = 0;
        for (i, c) in cards.iter().enumerate() {
            assert!(c.0 < 52, "partial: card at index {i} has out-of-range id {}", c.0);
            let bit = 1u64 << c.0;
            assert!(mask & bit == 0, "partial: duplicate card {c:?} at index {i}");
            mask |= bit;
        }
        let mut hand = Hand::new();
        for c in cards { hand = hand.add_card(c.0 as usize); }
        PartialHand { hand, mask, count: cards.len() as u8 }
    }

    fn rank_with(&self, partial: &PartialHand, extra: &[Card]) -> u16 {
        let mut mask = partial.mask;
        for (i, c) in extra.iter().enumerate() {
            assert!(c.0 < 52, "rank_with: extra card at index {i} has out-of-range id {}", c.0);
            let bit = 1u64 << c.0;
            assert!(mask & bit == 0, "rank_with: duplicate card {c:?} at extra index {i}");
            mask |= bit;
        }
        let total = partial.count as usize + extra.len();
        assert!((5..=7).contains(&total), "rank_with: {total} cards outside the 5..=7 range");

        let mut hand = partial.hand;
        for c in extra { hand = hand.add_card(c.0 as usize); }
        hand.evaluate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rank5;

    fn e() -> BinaryEvaluator { BinaryEvaluator }

    #[test]
    #[should_panic(expected = "out-of-range id 52")]
    fn partial_rejects_out_of_range_id() {
        e().partial(&[Card(52)]);
    }

    #[test]
    #[should_panic(expected = "duplicate card")]
    fn partial_rejects_duplicate_card() {
        e().partial(&[Card(0), Card(0)]);
    }

    #[test]
    #[should_panic(expected = "exceeds the 5-card board limit")]
    fn partial_rejects_oversized_board() {
        e().partial(&[Card(0), Card(4), Card(8), Card(12), Card(16), Card(20)]);
    }

    #[test]
    #[should_panic(expected = "duplicate card")]
    fn rank_with_rejects_duplicate_within_extra() {
        let ev = e();
        let p = ev.partial(&[]);
        ev.rank_with(&p, &[Card(0), Card(4), Card(8), Card(12), Card(0)]); // total 5, dup
    }

    #[test]
    #[should_panic(expected = "duplicate card")]
    fn rank_with_rejects_duplicate_across_partial_and_extra() {
        let ev = e();
        let p = ev.partial(&[Card(0)]);
        ev.rank_with(&p, &[Card(4), Card(8), Card(12), Card(0)]); // total 5, Card(0) repeats
    }

    #[test]
    #[should_panic(expected = "outside the 5..=7 range")]
    fn rank_with_rejects_undersized_total() {
        let ev = e();
        let p = ev.partial(&[]);
        ev.rank_with(&p, &[Card(0), Card(4), Card(8), Card(12)]); // total 4
    }

    #[test]
    #[should_panic(expected = "outside the 5..=7 range")]
    fn rank_with_rejects_oversized_total() {
        let ev = e();
        let p = ev.partial(&[]);
        ev.rank_with(
            &p,
            &[Card(0), Card(4), Card(8), Card(12), Card(16), Card(20), Card(24), Card(28)],
        ); // total 8
    }

    #[test]
    #[should_panic(expected = "out-of-range id 52")]
    fn rank_with_rejects_out_of_range_extra_id() {
        let ev = e();
        let p = ev.partial(&[Card(0), Card(4), Card(8), Card(12)]); // 4-card board
        ev.rank_with(&p, &[Card(52)]); // total would be 5, but the id is invalid
    }

    #[test]
    fn rank_with_accepts_a_valid_six_card_hand_regardless_of_extra_order() {
        let ev = e();
        let board = [Card(0), Card(4), Card(8)]; // 2c 3c 4c
        let p = ev.partial(&board);
        let extra = [Card(12), Card(16), Card(24)]; // 5c 6c 8c: total 6
        let r = ev.rank_with(&p, &extra);
        let r_reordered = ev.rank_with(&p, &[Card(24), Card(12), Card(16)]);
        assert_eq!(r, r_reordered, "rank must not depend on extra-card order");
    }

    #[test]
    fn rank_with_reuses_a_partial_across_several_extra_sets_and_matches_rank5() {
        let ev = e();
        let board = [Card(0), Card(4), Card(8), Card(12)]; // 2c 3c 4c 5c (4-card partial)
        let p = ev.partial(&board);
        for hole in [Card(16), Card(20), Card(24)] {
            // 6c, 7c, 8c in turn: reuse the same partial, total 5 each time
            let via_partial = ev.rank_with(&p, &[hole]);
            let mut all = board.to_vec();
            all.push(hole);
            let direct = rank5(&all.try_into().unwrap());
            assert_eq!(via_partial, direct, "reusing partial must match a direct rank5 call for {hole:?}");
        }
    }
}
