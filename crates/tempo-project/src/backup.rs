//! Rolling project backups, kept the way DaVinci Resolve keeps them: several from the
//! last hour, one per hour, and the last one of each day. Every backup is a complete
//! project file that is checked before it is kept.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Timelike, Utc};
use tempo_timeline::Project;

use crate::error::{ProjectError, Result};
use crate::project::{load_project, save_project};

/// Backup files are named by the time in UTC, so the hour repeated when daylight saving
/// ends can't make a newer backup look older. They are shown and kept by local time.
const NAME_FORMAT: &str = "%Y%m%d-%H%M%SZ";
const EXTENSION: &str = "tempo";

/// How often to back up and how many to keep. The defaults are Resolve's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupPolicy {
    pub every_min: u32,
    /// Backups from the last hour.
    pub keep_recent: usize,
    /// The newest backup of each hour.
    pub keep_hourly: usize,
    /// The newest backup of each day.
    pub keep_daily: usize,
}

impl Default for BackupPolicy {
    fn default() -> Self {
        Self { every_min: 10, keep_recent: 6, keep_hourly: 8, keep_daily: 5 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupEntry {
    pub path: PathBuf,
    pub taken: DateTime<Local>,
}

/// Write a backup of `project` into `dir`, check it, then drop the backups `policy` no
/// longer keeps. Blocking: call from a worker thread.
pub fn write_backup(project: &Project, dir: &Path, now: DateTime<Local>, policy: &BackupPolicy) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = format!("{}.{EXTENSION}", now.with_timezone(&Utc).format(NAME_FORMAT));
    let target = dir.join(&name);
    // Hidden while unfinished, so a half-written file is never listed as a backup.
    let partial = dir.join(format!(".{name}.part"));
    let written = save_project(project, &partial).and_then(|()| verify_backup(&partial));
    if let Err(e) = written {
        remove_with_sidecars(&partial);
        return Err(e);
    }
    std::fs::rename(&partial, &target)?;
    for old in to_prune(&list_backups(dir)?, now, policy) {
        remove_with_sidecars(&old);
    }
    Ok(target)
}

/// The project id stored in a file, read without loading the rest, so it works on a
/// project whose clips can no longer be read.
pub fn read_project_id(path: &Path) -> Result<uuid::Uuid> {
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let id: String = conn.query_row("SELECT id FROM project_meta LIMIT 1", [], |r| r.get(0))?;
    Ok(uuid::Uuid::parse_str(&id)?)
}

/// `write_backup` now, with Resolve's default policy.
pub fn backup_now(project: &Project, dir: &Path) -> Result<PathBuf> {
    write_backup(project, dir, Local::now(), &BackupPolicy::default())
}

/// A backup is good when SQLite finds nothing wrong and the whole project loads.
pub fn verify_backup(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Err(ProjectError::InvalidProject(format!("{} does not exist", path.display())));
    }
    let check: String = rusqlite::Connection::open(path)?.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(ProjectError::InvalidProject(format!("integrity check failed: {check}")));
    }
    load_project(path).map(|_| ())
}

/// The backups in `dir`, newest first. Files that are not backups are ignored.
pub fn list_backups(dir: &Path) -> Result<Vec<BackupEntry>> {
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
        Err(e) => return Err(e.into()),
    };
    for item in read {
        let path = item?.path();
        if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        if let Ok(utc) = NaiveDateTime::parse_from_str(stem, NAME_FORMAT) {
            entries.push(BackupEntry { path, taken: Utc.from_utc_datetime(&utc).with_timezone(&Local) });
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.taken));
    Ok(entries)
}

/// The backups `policy` no longer keeps at `now`. `existing` must be newest first.
pub fn to_prune(existing: &[BackupEntry], now: DateTime<Local>, policy: &BackupPolicy) -> Vec<PathBuf> {
    let mut keep: HashSet<&Path> = HashSet::new();
    let recent = existing.iter().filter(|e| now - e.taken < chrono::Duration::hours(1)).take(policy.keep_recent);
    keep.extend(recent.map(|e| e.path.as_path()));
    // Hourly backups are for the day being worked on; older days keep one each.
    let today = existing.iter().filter(|e| e.taken.date_naive() == now.date_naive());
    keep.extend(newest_per_bucket(today, |e| e.taken.hour(), policy.keep_hourly));
    keep.extend(newest_per_bucket(existing.iter(), |e| e.taken.date_naive(), policy.keep_daily));
    existing.iter().filter(|e| !keep.contains(e.path.as_path())).map(|e| e.path.clone()).collect()
}

/// The first entry of each of the first `count` buckets, from entries sorted newest first.
fn newest_per_bucket<'a, K: PartialEq>(entries: impl Iterator<Item = &'a BackupEntry>, key: impl Fn(&BackupEntry) -> K, count: usize) -> Vec<&'a Path> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for e in entries {
        let k = key(e);
        if seen.contains(&k) {
            continue;
        }
        if seen.len() == count {
            break;
        }
        seen.push(k);
        out.push(e.path.as_path());
    }
    out
}

fn remove_with_sidecars(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::{Clip, ClipType, RationalFps};
    use uuid::Uuid;

    fn at(s: &str) -> DateTime<Local> {
        Local.from_local_datetime(&NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()).earliest().unwrap()
    }

    fn entries(times: &[&str]) -> Vec<BackupEntry> {
        let mut v: Vec<_> = times.iter().map(|t| BackupEntry { path: PathBuf::from(*t), taken: at(t) }).collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.taken));
        v
    }

    fn kept(existing: &[BackupEntry], now: &str) -> Vec<String> {
        let pruned = to_prune(existing, at(now), &BackupPolicy::default());
        let mut k: Vec<_> = existing.iter().filter(|e| !pruned.contains(&e.path)).map(|e| e.path.display().to_string()).collect();
        k.sort();
        k
    }

    #[test]
    fn prune_keeps_six_recent_within_last_hour() {
        let times = ["2026-10-20 10:00", "2026-10-20 10:10", "2026-10-20 10:20", "2026-10-20 10:30", "2026-10-20 10:40", "2026-10-20 10:50", "2026-10-20 10:55"];
        let k = kept(&entries(&times), "2026-10-20 10:58");
        // 10:00 is the oldest of seven in the hour; the hourly rule keeps only 10:55 for hour 10.
        assert_eq!(k.len(), 6);
        assert!(!k.contains(&"2026-10-20 10:00".to_string()));
    }

    #[test]
    fn prune_keeps_newest_of_each_hour_up_to_eight() {
        let times: Vec<String> = (0..12).flat_map(|h| [format!("2026-10-20 {h:02}:05"), format!("2026-10-20 {h:02}:45")]).collect();
        let refs: Vec<&str> = times.iter().map(|s| s.as_str()).collect();
        let k = kept(&entries(&refs), "2026-10-20 13:30");
        // Nothing is within the last hour; hours 04..=11 keep their :45 backup.
        let want: Vec<String> = (4..12).map(|h| format!("2026-10-20 {h:02}:45")).collect();
        assert_eq!(k, want);
    }

    #[test]
    fn prune_keeps_last_backup_of_each_day_for_five_days() {
        // Days need not be consecutive.
        let times = ["2026-10-01 18:00", "2026-10-03 09:00", "2026-10-03 17:00", "2026-10-07 12:00", "2026-10-10 12:00", "2026-10-12 12:00", "2026-10-15 12:00"];
        let k = kept(&entries(&times), "2026-10-20 09:00");
        assert_eq!(k, ["2026-10-03 17:00", "2026-10-07 12:00", "2026-10-10 12:00", "2026-10-12 12:00", "2026-10-15 12:00"]);
    }

    #[test]
    fn prune_never_keeps_more_than_nineteen() {
        let times: Vec<String> = (1..=9).flat_map(|d| (0..24).flat_map(move |h| (0..6).map(move |m| format!("2026-10-{d:02} {h:02}:{:02}", m * 10)))).collect();
        let refs: Vec<&str> = times.iter().map(|s| s.as_str()).collect();
        assert!(kept(&entries(&refs), "2026-10-09 23:55").len() <= 19);
    }

    fn sample_project() -> Project {
        let mut project = Project::new("Backup", 1920, 1080, RationalFps::FPS_30);
        let track = project.timeline.tracks[0].id;
        project.timeline.tracks[0].clips.push(Clip::new(track, Uuid::new_v4(), ClipType::Video, "c", 0, 1_000_000, 0, 1_000_000));
        project
    }

    #[test]
    fn backup_round_trips_and_lists() {
        let dir = tempfile::tempdir().unwrap();
        let project = sample_project();
        let path = write_backup(&project, dir.path(), at("2026-10-20 10:00"), &BackupPolicy::default()).unwrap();
        let loaded = load_project(&path).unwrap();
        assert_eq!(loaded.timeline, project.timeline);
        let listed = list_backups(dir.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, path);
        assert_eq!(listed[0].taken, at("2026-10-20 10:00"), "the time read back from the name is the time taken");
        assert!(path.to_string_lossy().ends_with("Z.tempo"), "named in UTC");
        // Only the finished backup remains: no partial file or SQLite sidecars.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn verify_rejects_truncated_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_backup(&sample_project(), dir.path(), at("2026-10-20 10:00"), &BackupPolicy::default()).unwrap();
        let len = std::fs::metadata(&path).unwrap().len();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(len / 2).unwrap();
        drop(file);
        assert!(verify_backup(&path).is_err());
    }

    #[test]
    fn failed_backup_leaves_nothing_behind() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        // Root (as in CI containers) can write to a read-only folder, so nothing can fail here.
        if std::fs::write(dir.path().join("probe"), b"").is_ok() {
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            return;
        }
        let result = write_backup(&sample_project(), dir.path(), at("2026-10-20 10:00"), &BackupPolicy::default());
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn verify_rejects_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.tempo");
        assert!(verify_backup(&missing).is_err());
        assert!(!missing.exists(), "checking must not create the file");
    }
}
