//! `core-preflop`: the normalized preflop source envelope (spec section 8.2) and the
//! `PreflopSource`/`PreflopNodeKey`/`PreflopStep`/`PreflopNode` interface (spec section 8.1)
//! that later plan-3 tasks implement against.

pub mod envelope;
pub mod validate;

pub use envelope::*;
pub use validate::*;
