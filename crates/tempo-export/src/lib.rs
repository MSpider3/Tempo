//! tempo-export: Video and audio encoding pipeline (H.264, AAC, EDL, FCPXML).

pub mod chapters;
pub mod edl;
pub mod error;
pub mod ffmpeg;
pub mod timeline_export;

pub use chapters::{chapters_from_markers, ffmetadata, format_chapter_list, youtube_problems, Chapter};
pub use edl::export_timeline_to_edl;
pub use timeline_export::{build_ffmpeg_args, export_timeline, Fit, TimelineExport};
pub use error::{ExportError, Result};

