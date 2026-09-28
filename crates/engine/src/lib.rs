//! Engine: decision identity, the time source every deadline is measured against, and the
//! solve-tree templates of spec section 10.1.

pub mod allin;
pub mod assemble;
pub mod bench_support;
pub mod clock;
pub mod core;
pub mod coverage;
pub mod deadline;
pub mod engine;
pub mod equity;
pub mod identity;
pub mod log;
pub mod ranges;
pub mod serve;
pub mod snapshots;
pub mod solve;
pub mod startup;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod tree;
pub mod watchdog;
pub mod worker;

pub use engine::{Engine, Paths};
pub use startup::StartupReport;

/// Where the engine delivers a request's `RecommendationEvent`s (spec 3.4: results flow to the UI through a
/// `Channel<RecommendationEvent>`; tests record them). Called from engine threads, hence `Send`.
pub trait EventSink: Send { fn emit(&mut self, ev: proto::RecommendationEvent); }

/// The zero `Assumptions` of §4.4: no ranges, no tree, nothing measured yet.
pub fn assumptions_stub() -> proto::Assumptions {
    proto::Assumptions { ranges_used: vec![], tree_signature: String::new(), template_id: String::new(), source: String::new(), source_accuracy: "unverified".into(),
        source_granularity: "1326 combos".into(), target_bp: 50, reached_bp: None, elapsed_ms: 0, cache: "miss".into(), translations: vec![], mappings: vec![], notes: vec![] }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine error: {0}")]
    Message(String),
    #[error("rules: {0}")]
    Rules(String),
    #[error("overflow: pot + stacks must stay below 2^31")]
    Overflow,
}
