//! tempo-timeline: Core domain types, timeline model, and command-based undo/redo.

pub mod error;
pub mod types;
pub mod command;

pub use error::{Result, TimelineError};
pub use types::*;
pub use command::*;
