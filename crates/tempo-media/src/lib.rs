//! tempo-media: Media import, FFmpeg probe, decode, and thumbnailing.

pub mod decoder;
pub mod error;
pub mod hwaccel;
pub mod probe;

pub use decoder::{AudioBuffer, FfmpegDecoder, PixelFormat, VideoFrame, SEEK_THRESHOLD_US};
pub use error::{MediaError, Result};
pub use hwaccel::VaapiHwContext;
pub use probe::{ensure_ffmpeg_init, AudioMetadata, MediaEngine, MediaMetadata, VideoMetadata};
