//! On-disk street-solution cache: canonical structural keys (this task) plus, in later
//! tasks, the bincode+zstd store and background pre-solver queue (spec section 10.4-10.5).

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("invalid cache value: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Codec(#[from] bincode::Error),
}

pub mod entry;
pub mod key;
pub mod label;
pub mod lookup;
pub mod storage;
