//! tempo-media: probing and decoding through FFmpeg.

pub mod audio;
pub mod decoder;
pub mod error;
pub mod probe;

pub use audio::{waveform_peaks, AudioReader, OUT_CHANNELS, OUT_RATE};
pub use decoder::{set_hardware_decode, FfmpegDecoder, VideoFrame, SEEK_THRESHOLD_US};
pub use error::{MediaError, Result};
pub use probe::{ensure_ffmpeg_init, AudioMetadata, MediaEngine, MediaMetadata, VideoMetadata};
