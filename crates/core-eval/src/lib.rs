pub mod evaluator;

pub use evaluator::{BinaryEvaluator, Evaluator, PartialHand};
use proto::Card;

pub fn rank(cards: &[Card]) -> u16 { BinaryEvaluator.rank(cards) }
pub fn rank5(cards: &[Card; 5]) -> u16 { BinaryEvaluator.rank(cards) }
pub fn rank7(cards: &[Card; 7]) -> u16 { BinaryEvaluator.rank(cards) }
