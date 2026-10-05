use thiserror::Error;

#[derive(Error, Debug)]
pub enum ExportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("FFmpeg execution error: {0}")]
    Ffmpeg(String),

    #[error("Invalid timeline or parameters: {0}")]
    InvalidParameter(String),
}

pub type Result<T> = std::result::Result<T, ExportError>;
