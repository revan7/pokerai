//! Solve-tree construction: the templates of spec section 10.1, the materializer that builds one of
//! them against a concrete street root (§4.6), the effective tree assembled from a
//! `StreetRootSnapshot` (§10.2), its structural signature and node addressing.

pub mod effective;
pub mod materialize;
pub mod resolve;
pub mod signature;
pub mod templates;
pub use effective::{build_effective_tree, build_tree_full, materialize_at, TemplateSelection, TreeBuild, RULES_VERSION};
pub use resolve::{node_at, resolve_chip_path};
pub use signature::tree_signature;
pub use templates::{TemplateSpec, Templates};
