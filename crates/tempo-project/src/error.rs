use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProjectError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("UUID parse error: {0}")]
    Uuid(#[from] uuid::Error),

    #[error("Invalid project file: {0}")]
    InvalidProject(String),

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("Schema version mismatch: project is version {0}, supported is {1}")]
    UnsupportedSchemaVersion(u32, u32),
}

pub type Result<T> = std::result::Result<T, ProjectError>;
