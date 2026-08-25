use pult_schema::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("path not found: {0:?}")]
    PathNotFound(Path),
    #[error("invalid value for path {path:?}: {reason}")]
    InvalidValue { path: Path, reason: String },
    #[error("showfile error: {0}")]
    Showfile(#[from] anyhow::Error),
    #[error("serialization: {0}")]
    Json(#[from] serde_json::Error),
    #[error("channel closed")]
    ChannelClosed,
}

impl From<tokio::sync::oneshot::error::RecvError> for BackendError {
    fn from(_: tokio::sync::oneshot::error::RecvError) -> Self {
        BackendError::ChannelClosed
    }
}
