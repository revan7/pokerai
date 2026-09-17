//! Single source of truth for every serde type shared by the UI, the engine and the worker.

pub mod cards;
pub mod game;
pub mod hand;
pub use cards::*;
pub use game::*;
pub use hand::*;

/// Wire protocol version reported in `ready` (spec section 4.5).
pub const PROTO_VERSION: u16 = 3;

#[cfg(test)]
mod tests {
    #[test]
    fn proto_version_is_three() {
        assert_eq!(super::PROTO_VERSION, 3);
        assert!(cfg!(target_feature = "avx2"), "avx2 must be enabled by .cargo/config.toml");
    }
}
