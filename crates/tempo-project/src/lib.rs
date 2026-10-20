//! tempo-project: SQLite schema, migrations, and project persistence for Tempo 2.

pub mod error;
pub mod migration;
pub mod db;
pub mod project;
pub mod backup;

pub use error::{ProjectError, Result};
pub use db::{open_connection, get_journal_mode, get_foreign_keys_enabled};
pub use project::{
    create_new_project, save_project, load_project,
    autosave_path, auto_save_project, check_autosave_recovery, remove_autosave,
};
pub use migration::{CURRENT_SCHEMA_VERSION, apply_migrations};
pub use backup::{backup_now, list_backups, to_prune, verify_backup, write_backup, BackupEntry, BackupPolicy};
