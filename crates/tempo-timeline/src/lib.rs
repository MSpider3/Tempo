//! tempo-timeline: Core domain types, timeline model, and command-based undo/redo.

pub mod effects;
pub mod error;
pub mod types;
pub mod command;

pub use effects::{resolve as resolve_effects, Amount, ClipEffect, ColorTransform, EffectOp, EffectParam, ResolvedEffects};
pub use error::{Result, TimelineError};
pub use types::*;
pub use command::*;
