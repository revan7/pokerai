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

impl Evaluator for BinaryEvaluator {
    type Partial = Hand;
    fn partial(&self, cards: &[Card]) -> Hand {
        let ids: Vec<usize> = cards.iter().map(|c| c.0 as usize).collect();
        Hand::from_slice(&ids)
    }
    fn rank_with(&self, partial: &Hand, extra: &[Card]) -> u16 {
        let mut hand = *partial;
        for c in extra { hand = hand.add_card(c.0 as usize); }
        debug_assert!((5..=7).contains(&hand.len()), "evaluate needs 5 to 7 cards");
        hand.evaluate()
    }
}
