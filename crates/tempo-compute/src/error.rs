use thiserror::Error;

#[derive(Debug, Error)]
pub enum ComputeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Server failed to start within timeout")]
    ServerStartTimeout,
    #[error("RPC error ({code}): {message}")]
    Rpc { code: i64, message: String },
    #[error("Protocol error: {0}")]
    Protocol(String),
}
