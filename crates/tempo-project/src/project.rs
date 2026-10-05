use std::collections::HashMap;
use std::path::{Path, PathBuf};
use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;
use tempfile::NamedTempFile;

use tempo_timeline::types::{
    Clip, ClipProperties, ClipType, Marker, MarkerColor, MediaSource, MediaType,
    Project, RationalFps, TitleData, Track, TrackKind, Transition, TransitionAlignment,
    TransitionEdge, TransitionKind, UiState,
};

use crate::db::open_connection;
use crate::error::{ProjectError, Result};

pub fn create_new_project(
    name: &str,
    width: u32,
    height: u32,
    fps: RationalFps,
    path: &Path,
) -> Result<Project> {
    let now = Utc::now().timestamp_micros();
    let mut project = Project::new(name, width, height, fps);
    project.created_at = now;
    project.modified_at = now;

    save_project(&project, path)?;
    Ok(project)
}

pub fn save_project(project: &Project, path: &Path) -> Result<()> {
    let mut conn = open_connection(path)?;
    let tx = conn.transaction()?;

    let ui_state_json = serde_json::to_string(&project.ui_state)?;

    // 1. Save or update project_meta
    tx.execute(
        "INSERT INTO project_meta (
            id, name, created_at, modified_at, width, height,
            fps_num, fps_den, sample_rate, channels, proxy_dir, ui_state
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            modified_at = excluded.modified_at,
            width = excluded.width,
            height = excluded.height,
            fps_num = excluded.fps_num,
            fps_den = excluded.fps_den,
            sample_rate = excluded.sample_rate,
            channels = excluded.channels,
            proxy_dir = excluded.proxy_dir,
            ui_state = excluded.ui_state",
        params![
            project.id.to_string(),
            project.name,
            project.created_at,
            Utc::now().timestamp_micros(),
            project.width,
            project.height,
            project.fps.num,
            project.fps.den,
            project.sample_rate,
            project.channels,
            project.proxy_dir.to_string_lossy().to_string(),
            ui_state_json,
        ],
    )?;

    // 2. Clear dynamic tables to overwrite with current project state in transaction
    tx.execute("DELETE FROM media_sources", [])?;
    tx.execute("DELETE FROM tracks", [])?;
    tx.execute("DELETE FROM clips", [])?;
    tx.execute("DELETE FROM transitions", [])?;
    tx.execute("DELETE FROM markers", [])?;

    // 3. Save media sources
    for source in project.sources.values() {
        let media_type_int: i32 = match source.media_type {
            MediaType::Video => 0,
            MediaType::Audio => 1,
            MediaType::Image => 2,
        };

        let (fps_num, fps_den) = source.video_fps.map(|f| (Some(f.num), Some(f.den))).unwrap_or((None, None));

        tx.execute(
            "INSERT INTO media_sources (
                id, path, relative_path, media_type, duration_us, is_missing,
                video_width, video_height, video_fps_num, video_fps_den, video_codec,
                video_color_range, video_color_space, video_bit_depth, video_has_alpha,
                audio_sample_rate, audio_channels, audio_codec, audio_bit_rate,
                proxy_path, proxy_ready, imported_at, import_order, thumbnail_data
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                source.id.to_string(),
                source.path.to_string_lossy().to_string(),
                source.relative_path.to_string_lossy().to_string(),
                media_type_int,
                source.duration_us,
                if source.is_missing { 1 } else { 0 },
                source.video_width,
                source.video_height,
                fps_num,
                fps_den,
                source.video_codec,
                source.video_color_range.map(|v| v as i32),
                source.video_color_space.map(|v| v as i32),
                source.video_bit_depth.map(|v| v as i32),
                if source.video_has_alpha { 1 } else { 0 },
                source.audio_sample_rate,
                source.audio_channels,
                source.audio_codec,
                source.audio_bit_rate,
                source.proxy_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                if source.proxy_ready { 1 } else { 0 },
                source.imported_at,
                source.import_order,
                source.thumbnail_data,
            ],
        )?;
    }

    // 4. Save tracks, clips, transitions
    for track in &project.timeline.tracks {
        let kind_int = match track.kind {
            TrackKind::Video => 0,
            TrackKind::Audio => 1,
        };

        tx.execute(
            "INSERT INTO tracks (
                id, kind, kind_index, name, enabled, locked, solo, volume, height_px, color, sort_order
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                track.id.to_string(),
                kind_int,
                track.kind_index,
                track.name,
                if track.enabled { 1 } else { 0 },
                if track.locked { 1 } else { 0 },
                if track.solo { 1 } else { 0 },
                track.volume,
                track.height_px,
                track.color,
                track.sort_order,
            ],
        )?;

        // Clips on this track
        for clip in &track.clips {
            let clip_type_int = match clip.clip_type {
                ClipType::Video => 0,
                ClipType::Audio => 1,
                ClipType::Image => 2,
                ClipType::Title => 3,
            };

            let properties_json = serde_json::to_string(&clip.properties)?;
            let title_data_json = clip
                .title_data
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;

            tx.execute(
                "INSERT INTO clips (
                    id, track_id, source_id, clip_type, name,
                    timeline_in, timeline_out, source_in, source_out,
                    properties, title_data
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    clip.id.to_string(),
                    clip.track_id.to_string(),
                    clip.source_id.to_string(),
                    clip_type_int,
                    clip.name,
                    clip.timeline_in,
                    clip.timeline_out,
                    clip.source_in,
                    clip.source_out,
                    properties_json,
                    title_data_json,
                ],
            )?;
        }

        // Transitions on this track
        for trans in &track.transitions {
            let edge_int = match trans.edge {
                TransitionEdge::In => 0,
                TransitionEdge::Out => 1,
            };

            let (kind_int, plugin_id) = match &trans.kind {
                TransitionKind::Cut => (0, None),
                TransitionKind::CrossDissolve | TransitionKind::Crossfade => (1, None),
                TransitionKind::DipToBlack => (2, None),
                TransitionKind::DipToWhite => (3, None),
                TransitionKind::FadeIn | TransitionKind::FadeFromBlack => (4, None),
                TransitionKind::FadeOut | TransitionKind::FadeToBlack => (5, None),
                TransitionKind::FadeFromWhite => (7, None),
                TransitionKind::FadeToWhite => (8, None),
                TransitionKind::Plugin(pid) => (6, Some(pid.clone())),
            };

            let align_int = match trans.alignment {
                TransitionAlignment::Centered => 0,
                TransitionAlignment::StartAtCut => 1,
                TransitionAlignment::EndAtCut => 2,
            };

            tx.execute(
                "INSERT INTO transitions (
                    id, clip_id, edge, kind, duration_us, alignment, plugin_id, plugin_params
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    trans.id.to_string(),
                    trans.clip_id.to_string(),
                    edge_int,
                    kind_int,
                    trans.duration_us,
                    align_int,
                    plugin_id.or_else(|| trans.plugin_id.clone()),
                    trans.plugin_params.as_deref().unwrap_or("{}"),
                ],
            )?;
        }
    }

    // 5. Save markers
    for marker in &project.timeline.markers {
        let color_int = match marker.color {
            MarkerColor::Red => 0,
            MarkerColor::Green => 1,
            MarkerColor::Blue => 2,
            MarkerColor::Yellow => 3,
            MarkerColor::Orange => 4,
            MarkerColor::Purple => 5,
        };

        tx.execute(
            "INSERT INTO markers (
                id, position_us, name, color, duration_us, note
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                marker.id.to_string(),
                marker.position_us,
                marker.name,
                color_int,
                marker.duration_us,
                marker.note,
            ],
        )?;
    }

    tx.commit()?;
    Ok(())
}

pub fn load_project(path: &Path) -> Result<Project> {
    let conn = open_connection(path)?;

    // 1. Load project_meta
    let (id_str, name, created_at, modified_at, width, height, fps_num, fps_den, sample_rate, channels, proxy_dir_str, ui_state_json): (
        String, String, i64, i64, u32, u32, u32, u32, u32, u32, String, String
    ) = conn.query_row(
        "SELECT id, name, created_at, modified_at, width, height, fps_num, fps_den, sample_rate, channels, proxy_dir, ui_state
         FROM project_meta LIMIT 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
                row.get(11)?,
            ))
        },
    ).map_err(|e| ProjectError::InvalidProject(format!("Failed to load project metadata: {}", e)))?;

    let id = Uuid::parse_str(&id_str)?;
    let fps = RationalFps { num: fps_num, den: fps_den };
    let proxy_dir = PathBuf::from(proxy_dir_str);
    let ui_state: UiState = serde_json::from_str(&ui_state_json).unwrap_or_default();

    // 2. Load media sources
    let mut sources = HashMap::new();
    let mut source_stmt = conn.prepare(
        "SELECT id, path, relative_path, media_type, duration_us, is_missing,
                video_width, video_height, video_fps_num, video_fps_den, video_codec,
                video_color_range, video_color_space, video_bit_depth, video_has_alpha,
                audio_sample_rate, audio_channels, audio_codec, audio_bit_rate,
                proxy_path, proxy_ready, imported_at, import_order, thumbnail_data
         FROM media_sources ORDER BY import_order ASC",
    )?;

    let source_rows = source_stmt.query_map([], |row| {
        let id_str: String = row.get(0)?;
        let path_str: String = row.get(1)?;
        let rel_path_str: String = row.get(2)?;
        let media_type_int: i32 = row.get(3)?;
        let duration_us: i64 = row.get(4)?;
        let is_missing: i64 = row.get(5)?;
        let video_width: Option<u32> = row.get(6)?;
        let video_height: Option<u32> = row.get(7)?;
        let video_fps_num: Option<u32> = row.get(8)?;
        let video_fps_den: Option<u32> = row.get(9)?;
        let video_codec: Option<String> = row.get(10)?;
        let video_color_range: Option<u8> = row.get(11)?;
        let video_color_space: Option<u8> = row.get(12)?;
        let video_bit_depth: Option<u8> = row.get(13)?;
        let video_has_alpha: i64 = row.get(14)?;
        let audio_sample_rate: Option<u32> = row.get(15)?;
        let audio_channels: Option<u32> = row.get(16)?;
        let audio_codec: Option<String> = row.get(17)?;
        let audio_bit_rate: Option<u32> = row.get(18)?;
        let proxy_path_str: Option<String> = row.get(19)?;
        let proxy_ready: i64 = row.get(20)?;
        let imported_at: i64 = row.get(21)?;
        let import_order: u32 = row.get(22)?;
        let thumbnail_data: Option<Vec<u8>> = row.get(23)?;

        let media_type = match media_type_int {
            0 => MediaType::Video,
            1 => MediaType::Audio,
            _ => MediaType::Image,
        };

        let video_fps = match (video_fps_num, video_fps_den) {
            (Some(num), Some(den)) => Some(RationalFps { num, den }),
            _ => None,
        };

        Ok(MediaSource {
            id: Uuid::parse_str(&id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
            path: PathBuf::from(path_str),
            relative_path: PathBuf::from(rel_path_str),
            media_type,
            duration_us,
            is_missing: is_missing != 0,
            video_width,
            video_height,
            video_fps,
            video_codec,
            video_color_range,
            video_color_space,
            video_bit_depth,
            video_has_alpha: video_has_alpha != 0,
            audio_sample_rate,
            audio_channels,
            audio_codec,
            audio_bit_rate,
            proxy_path: proxy_path_str.map(PathBuf::from),
            proxy_ready: proxy_ready != 0,
            thumbnail_data,
            imported_at,
            import_order,
        })
    })?;

    for src in source_rows {
        let src = src?;
        sources.insert(src.id, src);
    }

    // 3. Load tracks
    let mut tracks = Vec::new();
    let mut track_stmt = conn.prepare(
        "SELECT id, kind, kind_index, name, enabled, locked, solo, volume, height_px, color, sort_order
         FROM tracks ORDER BY sort_order ASC",
    )?;

    let track_rows = track_stmt.query_map([], |row| {
        let id_str: String = row.get(0)?;
        let kind_int: i32 = row.get(1)?;
        let kind_index: u32 = row.get(2)?;
        let name: String = row.get(3)?;
        let enabled: i64 = row.get(4)?;
        let locked: i64 = row.get(5)?;
        let solo: i64 = row.get(6)?;
        let volume: f32 = row.get(7)?;
        let height_px: u32 = row.get(8)?;
        let color: String = row.get(9)?;
        let sort_order: u32 = row.get(10)?;

        let kind = match kind_int {
            0 => TrackKind::Video,
            _ => TrackKind::Audio,
        };

        Ok(Track {
            id: Uuid::parse_str(&id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
            kind,
            kind_index,
            name,
            enabled: enabled != 0,
            locked: locked != 0,
            solo: solo != 0,
            volume,
            height_px,
            color,
            sort_order,
            clips: Vec::new(),
            transitions: Vec::new(),
        })
    })?;

    for trk in track_rows {
        tracks.push(trk?);
    }

    // 4. Load clips
    let mut clip_stmt = conn.prepare(
        "SELECT id, track_id, source_id, clip_type, name,
                timeline_in, timeline_out, source_in, source_out,
                properties, title_data
         FROM clips ORDER BY timeline_in ASC",
    )?;

    let clip_rows = clip_stmt.query_map([], |row| {
        let id_str: String = row.get(0)?;
        let track_id_str: String = row.get(1)?;
        let source_id_str: String = row.get(2)?;
        let clip_type_int: i32 = row.get(3)?;
        let name: String = row.get(4)?;
        let timeline_in: i64 = row.get(5)?;
        let timeline_out: i64 = row.get(6)?;
        let source_in: i64 = row.get(7)?;
        let source_out: i64 = row.get(8)?;
        let prop_json: String = row.get(9)?;
        let title_json: Option<String> = row.get(10)?;

        let clip_type = match clip_type_int {
            0 => ClipType::Video,
            1 => ClipType::Audio,
            2 => ClipType::Image,
            _ => ClipType::Title,
        };

        let properties: ClipProperties = serde_json::from_str(&prop_json).unwrap_or_default();
        let title_data: Option<TitleData> = title_json.and_then(|s| serde_json::from_str(&s).ok());

        Ok(Clip {
            id: Uuid::parse_str(&id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
            track_id: Uuid::parse_str(&track_id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e)))?,
            source_id: Uuid::parse_str(&source_id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e)))?,
            clip_type,
            name,
            timeline_in,
            timeline_out,
            source_in,
            source_out,
            properties,
            title_data,
        })
    })?;

    for clip in clip_rows {
        let clip = clip?;
        if let Some(track) = tracks.iter_mut().find(|t| t.id == clip.track_id) {
            track.clips.push(clip);
        }
    }

    // 5. Load transitions
    let mut trans_stmt = conn.prepare(
        "SELECT id, clip_id, edge, kind, duration_us, alignment, plugin_id, plugin_params
         FROM transitions",
    )?;

    let trans_rows = trans_stmt.query_map([], |row| {
        let id_str: String = row.get(0)?;
        let clip_id_str: String = row.get(1)?;
        let edge_int: i32 = row.get(2)?;
        let kind_int: i32 = row.get(3)?;
        let duration_us: i64 = row.get(4)?;
        let align_int: i32 = row.get(5)?;
        let plugin_id: Option<String> = row.get(6)?;
        let plugin_params: Option<String> = row.get(7)?;

        let edge = if edge_int == 0 {
            TransitionEdge::In
        } else {
            TransitionEdge::Out
        };

        let kind = match kind_int {
            0 => TransitionKind::Cut,
            1 => TransitionKind::CrossDissolve,
            2 => TransitionKind::DipToBlack,
            3 => TransitionKind::DipToWhite,
            4 => TransitionKind::FadeIn,
            5 => TransitionKind::FadeOut,
            7 => TransitionKind::FadeFromWhite,
            8 => TransitionKind::FadeToWhite,
            _ => TransitionKind::Plugin(plugin_id.clone().unwrap_or_default()),
        };

        let alignment = match align_int {
            0 => TransitionAlignment::Centered,
            1 => TransitionAlignment::StartAtCut,
            _ => TransitionAlignment::EndAtCut,
        };

        Ok(Transition {
            id: Uuid::parse_str(&id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
            clip_id: Uuid::parse_str(&clip_id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e)))?,
            edge,
            kind,
            duration_us,
            alignment,
            plugin_id,
            plugin_params,
        })
    })?;

    for trans in trans_rows {
        let trans = trans?;
        // Find which track this clip belongs to
        for track in &mut tracks {
            if track.clips.iter().any(|c| c.id == trans.clip_id) {
                track.transitions.push(trans);
                break;
            }
        }
    }

    // 6. Load markers
    let mut markers = Vec::new();
    let mut marker_stmt = conn.prepare(
        "SELECT id, position_us, name, color, duration_us, note
         FROM markers ORDER BY position_us ASC",
    )?;

    let marker_rows = marker_stmt.query_map([], |row| {
        let id_str: String = row.get(0)?;
        let position_us: i64 = row.get(1)?;
        let name: String = row.get(2)?;
        let color_int: i32 = row.get(3)?;
        let duration_us: i64 = row.get(4)?;
        let note: String = row.get(5)?;

        let color = match color_int {
            0 => MarkerColor::Red,
            1 => MarkerColor::Green,
            2 => MarkerColor::Blue,
            3 => MarkerColor::Yellow,
            4 => MarkerColor::Orange,
            _ => MarkerColor::Purple,
        };

        Ok(Marker {
            id: Uuid::parse_str(&id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
            position_us,
            name,
            color,
            duration_us,
            note,
        })
    })?;

    for m in marker_rows {
        markers.push(m?);
    }

    let timeline = tempo_timeline::types::Timeline { tracks, markers };

    Ok(Project {
        id,
        name,
        created_at,
        modified_at,
        width,
        height,
        fps,
        sample_rate,
        channels,
        proxy_dir,
        timeline,
        sources,
        ui_state,
    })
}

/// Computes the auto-save file path for a project file.
/// E.g. `/path/to/my_project.tempo` -> `/path/to/.my_project.tempo.autosave`
pub fn autosave_path(project_path: &Path) -> PathBuf {
    let parent = project_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = project_path.file_name().and_then(|n| n.to_str()).unwrap_or("project.tempo");
    parent.join(format!(".{}.autosave", file_name))
}

/// Atomically writes an auto-save file for the project.
/// Writes to a temporary file via NamedTempFile in the same directory, then persists it to autosave_path.
pub fn auto_save_project(project: &Project, project_path: &Path) -> Result<PathBuf> {
    let auto_path = autosave_path(project_path);
    let parent = project_path.parent().unwrap_or_else(|| Path::new("."));

    let temp_file = NamedTempFile::new_in(parent)?;

    save_project(project, temp_file.path())?;

    temp_file
        .persist(&auto_path)
        .map_err(|e| ProjectError::Io(e.error))?;

    Ok(auto_path)
}

/// Checks if an auto-save file exists and is newer than the main project file (indicating a crash or unsaved changes).
pub fn check_autosave_recovery(project_path: &Path) -> Option<PathBuf> {
    let auto_path = autosave_path(project_path);
    if !auto_path.exists() {
        return None;
    }

    if !project_path.exists() {
        return Some(auto_path);
    }

    let main_meta = std::fs::metadata(project_path).ok()?;
    let auto_meta = std::fs::metadata(&auto_path).ok()?;

    let main_mtime = main_meta.modified().ok()?;
    let auto_mtime = auto_meta.modified().ok()?;

    if auto_mtime > main_mtime {
        Some(auto_path)
    } else {
        None
    }
}

/// Cleans up the autosave file after a successful explicit project save.
pub fn remove_autosave(project_path: &Path) {
    let auto_path = autosave_path(project_path);
    if auto_path.exists() {
        let _ = std::fs::remove_file(auto_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;


    #[test]
    fn test_create_and_load_empty_project() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();

        let proj = create_new_project("Test Project", 1920, 1080, RationalFps::FPS_30, path)
            .expect("create project");

        let loaded = load_project(path).expect("load project");
        assert_eq!(proj.id, loaded.id);
        assert_eq!(proj.name, loaded.name);
        assert_eq!(proj.width, loaded.width);
        assert_eq!(proj.height, loaded.height);
        assert_eq!(proj.fps, loaded.fps);
        assert_eq!(loaded.timeline.tracks.len(), 4);
    }

    #[test]
    fn test_wal_mode_enabled() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();

        let _ = create_new_project("WAL Project", 1920, 1080, RationalFps::FPS_24, path).unwrap();
        let conn = open_connection(path).unwrap();
        let mode = crate::db::get_journal_mode(&conn).unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn test_save_and_load_with_clips_tracks_markers_transitions() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();

        let mut project = Project::new("Complex Project", 3840, 2160, RationalFps::FPS_60);
        let src_id = Uuid::new_v4();
        let mut source = MediaSource::new(PathBuf::from("/media/video.mp4"), MediaType::Video, 60_000_000);
        source.id = src_id;
        source.video_width = Some(3840);
        source.video_height = Some(2160);
        source.video_fps = Some(RationalFps::FPS_60);
        project.sources.insert(src_id, source);

        // Add 10 clips across 3 tracks
        for track_idx in 0..3 {
            let track_id = project.timeline.tracks[track_idx].id;
            for clip_idx in 0..3 {
                let start = clip_idx * 5_000_000;
                let end = start + 4_000_000;
                let mut clip = Clip::new(
                    track_id,
                    src_id,
                    ClipType::Video,
                    format!("Clip_{}_{}", track_idx, clip_idx),
                    start,
                    end,
                    0,
                    4_000_000,
                );
                clip.properties.opacity = 0.85;
                project.timeline.tracks[track_idx].clips.push(clip);
            }
        }

        // Add a transition to track 0, clip 0
        let clip_0_id = project.timeline.tracks[0].clips[0].id;
        let trans = Transition {
            id: Uuid::new_v4(),
            clip_id: clip_0_id,
            edge: TransitionEdge::In,
            kind: TransitionKind::CrossDissolve,
            duration_us: 1_000_000,
            alignment: TransitionAlignment::Centered,
            plugin_id: None,
            plugin_params: None,
        };
        project.timeline.tracks[0].transitions.push(trans.clone());

        // Add markers
        let m1 = Marker::new(10_000_000, "Scene 1", MarkerColor::Red);
        let m2 = Marker::new(20_000_000, "Scene 2", MarkerColor::Green);
        project.timeline.add_marker(m1);
        project.timeline.add_marker(m2);

        // Save
        save_project(&project, path).expect("save should succeed");

        // Load
        let loaded = load_project(path).expect("load should succeed");
        assert_eq!(loaded.id, project.id);
        assert_eq!(loaded.sources.len(), 1);
        assert_eq!(loaded.sources[&src_id].video_width, Some(3840));
        assert_eq!(loaded.timeline.markers.len(), 2);
        assert_eq!(loaded.timeline.markers[0].name, "Scene 1");
        assert_eq!(loaded.timeline.markers[1].name, "Scene 2");

        // Verify tracks and clips
        for track_idx in 0..3 {
            assert_eq!(loaded.timeline.tracks[track_idx].clips.len(), 3);
            for clip_idx in 0..3 {
                let orig = &project.timeline.tracks[track_idx].clips[clip_idx];
                let loaded_clip = &loaded.timeline.tracks[track_idx].clips[clip_idx];
                assert_eq!(orig.id, loaded_clip.id);
                assert_eq!(orig.timeline_in, loaded_clip.timeline_in);
                assert_eq!(orig.timeline_out, loaded_clip.timeline_out);
                assert!((orig.properties.opacity - loaded_clip.properties.opacity).abs() < 0.001);
            }
        }

        // Verify transition
        assert_eq!(loaded.timeline.tracks[0].transitions.len(), 1);
        let loaded_trans = &loaded.timeline.tracks[0].transitions[0];
        assert_eq!(loaded_trans.id, trans.id);
        assert_eq!(loaded_trans.clip_id, clip_0_id);
        assert_eq!(loaded_trans.kind, TransitionKind::CrossDissolve);
    }

    #[test]
    fn test_full_project_round_trip() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();

        let mut project = Project::new("Full Round Trip Project", 1920, 1080, RationalFps::FPS_30);
        let src_id = Uuid::new_v4();
        let mut source = MediaSource::new(PathBuf::from("/media/video.mp4"), MediaType::Video, 100_000_000);
        source.id = src_id;
        source.video_width = Some(1920);
        source.video_height = Some(1080);
        source.video_fps = Some(RationalFps::FPS_30);
        project.sources.insert(src_id, source);

        let v1_track_id = project.timeline.tracks[0].id;
        let v2_track_id = project.timeline.tracks[1].id;

        // Add 10 clips across tracks
        for i in 0..10 {
            let track_id = if i % 2 == 0 { v1_track_id } else { v2_track_id };
            let start = (i as i64) * 3_000_000;
            let end = start + 2_500_000;
            let mut clip = Clip::new(
                track_id,
                src_id,
                ClipType::Video,
                format!("Clip_{:02}", i),
                start,
                end,
                0,
                2_500_000,
            );
            clip.properties.opacity = 0.9;
            clip.properties.volume = 0.8;
            if i % 2 == 0 {
                project.timeline.tracks[0].clips.push(clip);
            } else {
                project.timeline.tracks[1].clips.push(clip);
            }
        }

        // Add 3 markers
        let m1 = Marker::new(2_000_000, "Intro", MarkerColor::Red);
        let m2 = Marker::new(12_000_000, "Action", MarkerColor::Green);
        let m3 = Marker::new(25_000_000, "Outro", MarkerColor::Blue);
        project.timeline.add_marker(m1);
        project.timeline.add_marker(m2);
        project.timeline.add_marker(m3);

        // Add 2 transitions
        let c0_id = project.timeline.tracks[0].clips[0].id;
        let c1_id = project.timeline.tracks[0].clips[1].id;
        let t1 = Transition {
            id: Uuid::new_v4(),
            clip_id: c0_id,
            edge: TransitionEdge::In,
            kind: TransitionKind::CrossDissolve,
            duration_us: 1_000_000,
            alignment: TransitionAlignment::Centered,
            plugin_id: None,
            plugin_params: None,
        };
        let t2 = Transition {
            id: Uuid::new_v4(),
            clip_id: c1_id,
            edge: TransitionEdge::Out,
            kind: TransitionKind::DipToBlack,
            duration_us: 1_500_000,
            alignment: TransitionAlignment::Centered,
            plugin_id: None,
            plugin_params: None,
        };
        project.timeline.tracks[0].transitions.push(t1);
        project.timeline.tracks[0].transitions.push(t2);

        // Save
        save_project(&project, path).expect("save should succeed");

        // Load
        let loaded = load_project(path).expect("load should succeed");

        // Verify project meta
        assert_eq!(loaded.id, project.id);
        assert_eq!(loaded.name, project.name);
        assert_eq!(loaded.width, 1920);
        assert_eq!(loaded.height, 1080);
        assert_eq!(loaded.fps, RationalFps::FPS_30);

        // Verify 10 clips
        let total_clips: usize = loaded.timeline.tracks.iter().map(|t| t.clips.len()).sum();
        assert_eq!(total_clips, 10);
        for t in 0..2 {
            for (c_orig, c_load) in project.timeline.tracks[t].clips.iter().zip(&loaded.timeline.tracks[t].clips) {
                assert_eq!(c_orig.id, c_load.id);
                assert_eq!(c_orig.name, c_load.name);
                assert_eq!(c_orig.timeline_in, c_load.timeline_in);
                assert_eq!(c_orig.timeline_out, c_load.timeline_out);
                assert_eq!(c_orig.source_in, c_load.source_in);
                assert_eq!(c_orig.source_out, c_load.source_out);
                assert!((c_orig.properties.opacity - c_load.properties.opacity).abs() < 0.001);
                assert!((c_orig.properties.volume - c_load.properties.volume).abs() < 0.001);
            }
        }

        // Verify 3 markers
        assert_eq!(loaded.timeline.markers.len(), 3);
        assert_eq!(loaded.timeline.markers[0].name, "Intro");
        assert_eq!(loaded.timeline.markers[1].name, "Action");
        assert_eq!(loaded.timeline.markers[2].name, "Outro");

        // Verify 2 transitions
        assert_eq!(loaded.timeline.tracks[0].transitions.len(), 2);
        assert_eq!(loaded.timeline.tracks[0].transitions[0].kind, TransitionKind::CrossDissolve);
        assert_eq!(loaded.timeline.tracks[0].transitions[1].kind, TransitionKind::DipToBlack);
    }

    #[test]
    fn test_autosave_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let proj_path = dir.path().join("test_proj.tempo");

        let mut project = Project::new("Autosave Test", 1920, 1080, RationalFps::FPS_30);
        save_project(&project, &proj_path).unwrap();

        // Initially no autosave exists
        assert!(check_autosave_recovery(&proj_path).is_none());

        // Sleep briefly to ensure mtime increases
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Add an edit to project and trigger auto_save_project
        let track_id = project.timeline.tracks[0].id;
        let clip = Clip::new(
            track_id,
            Uuid::new_v4(),
            ClipType::Video,
            "Autosaved Clip",
            0,
            5_000_000,
            0,
            5_000_000,
        );
        project.timeline.tracks[0].clips.push(clip);

        let auto_path = auto_save_project(&project, &proj_path).expect("autosave should succeed");
        assert!(auto_path.exists());

        // Verify crash recovery detects newer autosave
        let recovery = check_autosave_recovery(&proj_path);
        assert!(recovery.is_some());
        let recovered_path = recovery.unwrap();
        assert_eq!(recovered_path, auto_path);

        // Load recovered project and check clip
        let recovered_proj = load_project(&recovered_path).unwrap();
        assert_eq!(recovered_proj.timeline.tracks[0].clips.len(), 1);
        assert_eq!(recovered_proj.timeline.tracks[0].clips[0].name, "Autosaved Clip");

        // Clean up autosave
        remove_autosave(&proj_path);
        assert!(!auto_path.exists());
        assert!(check_autosave_recovery(&proj_path).is_none());
    }

    #[test]
    fn test_inspector_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let proj_path = dir.path().join("inspector_test.tempo");

        let mut project = Project::new("Inspector Round Trip", 1920, 1080, RationalFps::FPS_24);
        let track_id = project.timeline.tracks[0].id;

        let mut clip = Clip::new(
            track_id,
            Uuid::new_v4(),
            ClipType::Video,
            "Inspector Clip",
            0,
            5_000_000,
            0,
            5_000_000,
        );

        // Set opacity=0.5, position=(100,-50), scale=(1.5,1.5), rotation=15.0 per Phase 3 requirements
        clip.properties.opacity = 0.5;
        clip.properties.position_x = 100.0;
        clip.properties.position_y = -50.0;
        clip.properties.scale_x = 1.5;
        clip.properties.scale_y = 1.5;
        clip.properties.rotation = 15.0;

        clip.properties.lift = [0.1, 0.05, 0.0];
        clip.properties.gamma = [1.2, 1.1, 1.0];
        clip.properties.gain = [1.5, 1.4, 1.3];
        clip.properties.add_keyframe("opacity", 1_000_000, 0.0);
        clip.properties.add_keyframe("opacity", 3_000_000, 1.0);

        project.timeline.tracks[0].clips.push(clip);

        save_project(&project, &proj_path).expect("Failed to save project");

        let loaded = load_project(&proj_path).expect("Failed to load project");
        assert_eq!(loaded.timeline.tracks[0].clips.len(), 1);
        let loaded_clip = &loaded.timeline.tracks[0].clips[0];

        assert_eq!(loaded_clip.properties.opacity, 0.5);
        assert_eq!(loaded_clip.properties.position_x, 100.0);
        assert_eq!(loaded_clip.properties.position_y, -50.0);
        assert_eq!(loaded_clip.properties.scale_x, 1.5);
        assert_eq!(loaded_clip.properties.scale_y, 1.5);
        assert_eq!(loaded_clip.properties.rotation, 15.0);

        assert_eq!(loaded_clip.properties.lift, [0.1, 0.05, 0.0]);
        assert_eq!(loaded_clip.properties.gamma, [1.2, 1.1, 1.0]);
        assert_eq!(loaded_clip.properties.gain, [1.5, 1.4, 1.3]);

        assert_eq!(loaded_clip.properties.keyframes.len(), 2);
        assert_eq!(loaded_clip.properties.evaluate_property("opacity", 2_000_000), 0.5);
    }
}
