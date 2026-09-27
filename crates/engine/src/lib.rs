//! Engine: decision identity, the time source every deadline is measured against, and the
//! solve-tree templates of spec section 10.1.

pub mod allin;
pub mod bench_support;
pub mod clock;
pub mod equity;
pub mod identity;
pub mod log;
pub mod tree;
pub mod worker;

/// Where the engine delivers a request's `RecommendationEvent`s (spec 3.4: results flow to the UI through a
/// `Channel<RecommendationEvent>`; tests record them). Called from engine threads, hence `Send`.
pub trait EventSink: Send { fn emit(&mut self, ev: proto::RecommendationEvent); }

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine error: {0}")]
    Message(String),
    #[error("rules: {0}")]
    Rules(String),
    #[error("overflow: pot + stacks must stay below 2^31")]
    Overflow,
}
