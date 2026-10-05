//! tempo-export: Video and audio encoding pipeline (H.264, AAC, EDL, FCPXML).

pub mod chapters;
pub mod edl;
pub mod error;
pub mod h264;
pub mod render_queue;
pub mod timeline_export;

pub use chapters::{chapters_from_markers, ffmetadata, format_chapter_list, youtube_problems, Chapter};
pub use edl::export_timeline_to_edl;
pub use timeline_export::{build_ffmpeg_args, export_timeline, Fit, TimelineExport};
pub use error::{ExportError, Result};
pub use h264::{export_raw_frames, ExportProgress, ExportSettings};
pub use render_queue::{
    is_hw_encoder_active, set_hw_encoder_active, ExportJob, ExportPreset, ExportQueueEvent,
    JobStatus, RenderQueue, RenderRange,
};

pub struct ExportEngine;

