//! tempo-project: SQLite schema, migrations, and project persistence for Tempo 2.

pub mod error;
pub mod migration;
pub mod db;
pub mod project;

pub use error::{ProjectError, Result};
pub use db::{open_connection, get_journal_mode, get_foreign_keys_enabled};
pub use project::{
    create_new_project, save_project, load_project,
    autosave_path, auto_save_project, check_autosave_recovery, remove_autosave,
};
pub use migration::{CURRENT_SCHEMA_VERSION, apply_migrations};
