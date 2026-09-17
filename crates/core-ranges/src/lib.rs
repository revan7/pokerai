mod parse;

pub use parse::{class_index, class_name, expand_169, parse_range, range_to_string};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RangeError {
    #[error("range syntax: {0}")]
    Syntax(String),
    #[error("range weight: {0}")]
    Weight(String),
}

/// Total weight (accumulated in f64).
pub fn mass(r: &proto::Range1326) -> f32 { r.0.iter().map(|w| *w as f64).sum::<f64>() as f32 }
