//! Engine: decision identity, the time source every deadline is measured against, and the
//! solve-tree templates of spec section 10.1.

pub mod clock;
pub mod identity;
pub mod tree;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine error: {0}")]
    Message(String),
    #[error("rules: {0}")]
    Rules(String),
    #[error("overflow: pot + stacks must stay below 2^31")]
    Overflow,
}
