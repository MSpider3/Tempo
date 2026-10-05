use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TimelineError {
    #[error("Track not found: {0}")]
    TrackNotFound(Uuid),

    #[error("Clip not found: {0}")]
    ClipNotFound(Uuid),

    #[error("Marker not found: {0}")]
    MarkerNotFound(Uuid),

    #[error("Media source not found: {0}")]
    SourceNotFound(Uuid),

    #[error("Invalid time range: in {0}us, out {1}us")]
    InvalidTimeRange(i64, i64),

    #[error("Clips overlap on track {0} at time {1}us")]
    ClipCollision(Uuid, i64),

    #[error("Track is locked: {0}")]
    TrackLocked(Uuid),

    #[error("Invalid split position {0}us for clip with range {1}us..{2}us")]
    InvalidSplitPosition(i64, i64, i64),

    #[error("Command execution failed: {0}")]
    CommandFailed(String),

    #[error("Nothing to undo")]
    NothingToUndo,

    #[error("Nothing to redo")]
    NothingToRedo,
}

pub type Result<T> = std::result::Result<T, TimelineError>;
