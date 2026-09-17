use serde::Serialize;

#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "type", content = "detail")]
pub enum AppError {
    #[error("engine: {message}")]
    Engine { message: String },
    #[error("command queue is full")]
    Busy,
    #[error("application is stopping")]
    Closed,
    #[error("serialization: {message}")]
    Serialization { message: String },
    #[error("unsupported in phase 1")]
    Unsupported { reason: proto::UnsupportedReason },
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization { message: e.to_string() }
    }
}
