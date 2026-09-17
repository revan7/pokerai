pub mod equity;
pub mod evaluator;
pub mod mc;

pub use equity::*;
pub use evaluator::{BinaryEvaluator, Evaluator, PartialHand};
pub use mc::{sample_joint_holes, Xoshiro256};
use proto::Card;

pub fn rank(cards: &[Card]) -> u16 { BinaryEvaluator.rank(cards) }
pub fn rank5(cards: &[Card; 5]) -> u16 { BinaryEvaluator.rank(cards) }
pub fn rank7(cards: &[Card; 7]) -> u16 { BinaryEvaluator.rank(cards) }
