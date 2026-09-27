//! `core-preflop`: the normalized preflop source envelope (spec section 8.2), the
//! `PreflopSource`/`PreflopNodeKey`/`PreflopStep`/`PreflopNode` interface (spec section 8.1), and
//! the lookup rules that turn a live decision into one source node: prefix reconstruction, depth
//! bucketing, rake-profile ranking, straddle virtual roles and short-handed mapping (section 8.3).
//! `translate` also assembles hero's current decision over the shared history branches (section
//! 8.4: `BranchNode`, `MixedNode`, `mix_action`, `mix_nodes`), re-exported at the crate root.

pub mod branches;
pub mod depth;
pub mod envelope;
pub mod ev;
pub mod lookup;
mod numeric;
pub mod store;
pub mod straddle;
pub mod translate;
pub mod validate;

pub use depth::*;
pub use envelope::*;
pub use ev::*;
pub use lookup::*;
pub use store::*;
pub use straddle::*;
pub use translate::*;
pub use validate::*;
