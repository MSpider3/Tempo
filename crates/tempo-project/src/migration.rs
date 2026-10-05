use chrono::Utc;
use rusqlite::Connection;
use crate::error::{ProjectError, Result};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

pub struct Migration {
    pub version: u32,
    pub description: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Initial schema",
        sql: include_str!("migrations/001_initial.sql"),
    },
];

pub fn apply_migrations(conn: &mut Connection) -> Result<()> {
    // Ensure schema_version table exists
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version     INTEGER NOT NULL,
            applied_at  INTEGER NOT NULL,
            description TEXT
        )",
        [],
    )?;

    let current_version: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if current_version > CURRENT_SCHEMA_VERSION {
        return Err(ProjectError::UnsupportedSchemaVersion(
            current_version,
            CURRENT_SCHEMA_VERSION,
        ));
    }

    for migration in MIGRATIONS.iter().filter(|m| m.version > current_version) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)?;
        let now_us = Utc::now().timestamp_micros();
        tx.execute(
            "INSERT INTO schema_version (version, applied_at, description) VALUES (?1, ?2, ?3)",
            rusqlite::params![migration.version, now_us, migration.description],
        )?;
        tx.commit()?;
        tracing::info!(
            version = migration.version,
            description = migration.description,
            "Applied schema migration"
        );
    }

    Ok(())
}
