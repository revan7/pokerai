use proto::CardParseError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RulesError {
    #[error("format unsupported: {detail}")]
    FormatUnsupported { detail: String },
    #[error("invalid config: {reason}")]
    InvalidConfig { reason: String },
    #[error("illegal action: {reason}")]
    IllegalAction { reason: String },
    #[error("no betting round is open")]
    NotBetting,
    #[error("the hand is not awaiting a board")]
    NotAwaitingBoard,
    #[error("bad board: {reason}")]
    BadBoard { reason: String },
    #[error("chip conservation violated: expected {expected}, found {actual}")]
    Conservation { expected: u64, actual: u64 },
    #[error(transparent)]
    Card(#[from] CardParseError),
}
