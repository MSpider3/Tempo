use std::path::Path;
use rusqlite::Connection;
use crate::error::Result;
use crate::migration::{apply_migrations, CURRENT_SCHEMA_VERSION};

pub fn open_connection(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut conn = Connection::open(path)?;

    // Set performance and durability pragmas
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "cache_size", -16000)?; // 16MB
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "foreign_keys", "OFF")?;

    backup_before_upgrade(&conn, path, CURRENT_SCHEMA_VERSION)?;
    apply_migrations(&mut conn)?;

    Ok(conn)
}

/// Copy a project written by an older Tempo to `<file>.v<version>.bak` before migrating
/// it, so a failed or unwanted upgrade can be undone by hand. New files are not copied.
fn backup_before_upgrade(conn: &Connection, path: &Path, target: u32) -> Result<Option<std::path::PathBuf>> {
    let version: u32 = conn
        .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_version", [], |r| r.get(0))
        .unwrap_or(0); // no table yet: a new file
    if version == 0 || version >= target {
        return Ok(None);
    }
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".v{version}.bak"));
    let backup = std::path::PathBuf::from(name);
    if !backup.exists() {
        // VACUUM INTO writes one consistent file, including changes still in the WAL.
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy()])?;
        tracing::info!(from = version, to = target, backup = %backup.display(), "backed up project before upgrading it");
    }
    Ok(Some(backup))
}


pub fn get_journal_mode(conn: &Connection) -> Result<String> {
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    Ok(mode.to_lowercase())
}

pub fn get_foreign_keys_enabled(conn: &Connection) -> Result<bool> {
    let enabled: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    Ok(enabled == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_old_schema_backs_up_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.tempo");
        let conn = open_connection(&path).unwrap();
        conn.execute("CREATE TABLE marker_check (x INTEGER)", []).unwrap();
        // Pretend a newer Tempo with schema 2 opens this version-1 file.
        let backup = backup_before_upgrade(&conn, &path, CURRENT_SCHEMA_VERSION + 1).unwrap().unwrap();
        let copy = Connection::open(&backup).unwrap();
        let tables: i64 = copy.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'marker_check'", [], |r| r.get(0)).unwrap();
        assert_eq!(tables, 1);
        // Current-version and brand-new files are never copied.
        assert!(backup_before_upgrade(&conn, &path, CURRENT_SCHEMA_VERSION).unwrap().is_none());
        let fresh = Connection::open(dir.path().join("new.tempo")).unwrap();
        assert!(backup_before_upgrade(&fresh, &dir.path().join("new.tempo"), 5).unwrap().is_none());
    }
}
