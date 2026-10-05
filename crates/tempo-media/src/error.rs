use thiserror::Error;

#[derive(Error, Debug)]
pub enum MediaError {
    #[error("FFmpeg error: {0}")]
    Ffmpeg(#[from] ffmpeg_next::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("File not found or unreadable: {0}")]
    FileNotFound(String),

    #[error("No valid video or audio stream found in: {0}")]
    NoStreamsFound(String),

    #[error("Failed to decode frame for thumbnail")]
    ThumbnailDecodeFailed,

    #[error("Failed to decode video frame")]
    FrameDecodeFailed,

    #[error("Failed to decode audio range")]
    AudioDecodeFailed,

    #[error("Seek failed to pts {0}us")]
    SeekFailed(i64),

    #[error("Source not found: {0}")]
    SourceNotFound(uuid::Uuid),

    #[error("Stream not found: {0}")]
    StreamNotFound(String),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
}

pub type Result<T> = std::result::Result<T, MediaError>;
