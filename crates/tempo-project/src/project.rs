use std::collections::HashMap;
use std::path::{Path, PathBuf};
use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;
use tempfile::NamedTempFile;

use tempo_timeline::types::{
    AudioChannelMap, AudioTrackType, Bin, Clip, ClipProperties, ClipType, Marker, MarkerColor,
    MediaSource, MediaType, Project, RationalFps, Timeline, TitleData, Track, TrackKind,
    Transition, TransitionAlignment, TransitionEase, TransitionEdge, TransitionKind, UiState,
    MASTER_BIN_ID,
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

    // A file holds one project: a copy saved under a new id must not keep the old row,
    // or loading could read it instead.
    tx.execute("DELETE FROM project_meta WHERE id <> ?1", params![project.id.to_string()])?;

    // 2. Clear dynamic tables to overwrite with current project state in transaction
    let master_bin_id = MASTER_BIN_ID.to_string();
    tx.execute("DELETE FROM media_sources", [])?;
    tx.execute("DELETE FROM bin_sources", [])?;
    tx.execute("DELETE FROM bins WHERE id <> ?1", params![master_bin_id])?;
    tx.execute(
        "INSERT OR IGNORE INTO bins (id, name, parent_id, sort_order) VALUES (?1, 'Master', NULL, 0)",
        params![master_bin_id],
    )?;
    tx.execute("DELETE FROM tracks", [])?;
    tx.execute("DELETE FROM clips", [])?;
    tx.execute("DELETE FROM transitions", [])?;
    tx.execute("DELETE FROM markers", [])?;
    tx.execute("DELETE FROM timelines", [])?;

    // 2b. Save Media Pool bins (excluding Master)
    for bin in &project.bins {
        if bin.id == MASTER_BIN_ID {
            continue;
        }
        let parent_id = bin.parent_id.filter(|id| *id != MASTER_BIN_ID).map(|id| id.to_string());
        tx.execute(
            "INSERT INTO bins (id, name, parent_id, sort_order) VALUES (?1, ?2, ?3, ?4)",
            params![bin.id.to_string(), bin.name, parent_id, bin.sort_order],
        )?;
    }

    // 3. Save media sources and their bin memberships
    for source in project.sources.values() {
        let media_type_int: i32 = match source.media_type {
            MediaType::Video => 0,
            MediaType::Audio => 1,
            MediaType::Image => 2,
        };

        let (fps_num, fps_den) = source.video_fps.map(|f| (Some(f.num), Some(f.den))).unwrap_or((None, None));
        let (cfps_num, cfps_den) = source.conform_fps.map(|f| (Some(f.num), Some(f.den))).unwrap_or((None, None));

        tx.execute(
            "INSERT INTO media_sources (
                id, path, relative_path, media_type, duration_us, is_missing,
                video_width, video_height, video_fps_num, video_fps_den, video_codec,
                video_color_range, video_color_space, video_bit_depth, video_has_alpha,
                audio_sample_rate, audio_channels, audio_codec, audio_bit_rate,
                proxy_path, proxy_ready, imported_at, import_order, thumbnail_data,
                display_name, conform_fps_num, conform_fps_den, audio_channel_map
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)",
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
                source.display_name,
                cfps_num,
                cfps_den,
                source.audio_channel_map.to_db_str(),
            ],
        )?;

        let bin_id = source.bin_id.filter(|id| *id != MASTER_BIN_ID).unwrap_or(MASTER_BIN_ID).to_string();
        tx.execute(
            "INSERT INTO bin_sources (bin_id, source_id, sort_order) VALUES (?1, ?2, ?3)",
            params![bin_id, source.id.to_string(), source.import_order as i32],
        )?;
    }

    // 4. Save every timeline: its tracks, clips, transitions and markers
    for (timeline, open) in std::iter::once((&project.timeline, true)).chain(project.timelines.iter().map(|t| (t, false))) {
    let timeline_id = timeline.id.to_string();
    let timeline_bin_id = timeline.bin_id.filter(|id| *id != MASTER_BIN_ID).map(|id| id.to_string());
    tx.execute(
        "INSERT INTO timelines (id, name, is_open, start_tc_us, width, height, fps_num, fps_den, bin_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            timeline_id,
            timeline.name,
            open as i32,
            timeline.start_tc_us,
            timeline.resolution.map(|r| r.0),
            timeline.resolution.map(|r| r.1),
            timeline.fps.map(|f| f.num),
            timeline.fps.map(|f| f.den),
            timeline_bin_id,
        ],
    )?;
    for track in &timeline.tracks {
        let kind_int = match track.kind {
            TrackKind::Video => 0,
            TrackKind::Audio => 1,
        };

        tx.execute(
            "INSERT INTO tracks (
                id, kind, kind_index, name, enabled, locked, solo, volume, height_px, color, sort_order, timeline_id, audio_type
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                timeline_id,
                matches!(track.audio_type, AudioTrackType::Stereo) as i32,
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
                TransitionKind::CrossDissolve => (1, None),
                TransitionKind::Crossfade => (10, None),
                TransitionKind::DipToBlack => (2, None),
                TransitionKind::DipToWhite => (3, None),
                TransitionKind::FadeIn | TransitionKind::FadeFromBlack => (4, None),
                TransitionKind::FadeOut | TransitionKind::FadeToBlack => (5, None),
                TransitionKind::FadeFromWhite => (7, None),
                TransitionKind::FadeToWhite => (8, None),
                TransitionKind::Plugin(pid) => (6, Some(pid.clone())),
                TransitionKind::DipToColor => (9, None),
                TransitionKind::AdditiveDissolve => (11, None),
                TransitionKind::NonAdditiveDissolve => (12, None),
                TransitionKind::BlurDissolve => (13, None),
            };

            let align_int = match trans.alignment {
                TransitionAlignment::Centered => 0,
                TransitionAlignment::StartAtCut => 1,
                TransitionAlignment::EndAtCut => 2,
            };

            let ease_int = match trans.ease {
                TransitionEase::None => 0,
                TransitionEase::In => 1,
                TransitionEase::Out => 2,
                TransitionEase::InOut => 3,
            };

            tx.execute(
                "INSERT INTO transitions (
                    id, clip_id, edge, kind, duration_us, alignment, plugin_id, plugin_params, color, ease,
                    start_ratio, end_ratio, curve, enabled
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    trans.id.to_string(),
                    trans.clip_id.to_string(),
                    edge_int,
                    kind_int,
                    trans.duration_us,
                    align_int,
                    plugin_id.or_else(|| trans.plugin_id.clone()),
                    trans.plugin_params.as_deref().unwrap_or("{}"),
                    color_to_hex(trans.color),
                    ease_int,
                    trans.start_ratio,
                    trans.end_ratio,
                    (!trans.curve.is_empty()).then(|| serde_json::to_string(&trans.curve)).transpose()?,
                    trans.enabled as i32,
                ],
            )?;
        }
    }

    // Markers
    for marker in &timeline.markers {
        let color_int = match marker.color {
            MarkerColor::Red => 0,
            MarkerColor::Green => 1,
            MarkerColor::Blue => 2,
            MarkerColor::Yellow => 3,
            MarkerColor::Orange => 4,
            MarkerColor::Purple => 5,
            MarkerColor::Cyan => 6,
            MarkerColor::Pink => 7,
            MarkerColor::Fuchsia => 8,
            MarkerColor::Rose => 9,
            MarkerColor::Lavender => 10,
            MarkerColor::Sky => 11,
            MarkerColor::Mint => 12,
            MarkerColor::Lemon => 13,
            MarkerColor::Sand => 14,
            MarkerColor::Cocoa => 15,
            MarkerColor::Cream => 16,
        };

        tx.execute(
            "INSERT INTO markers (
                id, position_us, name, color, duration_us, note, timeline_id, keyword
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                marker.id.to_string(),
                marker.position_us,
                marker.name,
                color_int,
                marker.duration_us,
                marker.note,
                timeline_id,
                marker.keyword,
            ],
        )?;
    }
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

    // 2. Load bins and source->bin memberships
    let master_bin_id = MASTER_BIN_ID.to_string();
    let mut bins = Vec::new();
    let mut bin_stmt = conn.prepare(
        "SELECT id, name, parent_id, sort_order FROM bins WHERE id <> ?1 ORDER BY sort_order ASC, rowid ASC",
    )?;
    let bin_rows = bin_stmt.query_map(params![master_bin_id], |row| {
        let id_str: String = row.get(0)?;
        let name: String = row.get(1)?;
        let parent_str: Option<String> = row.get(2)?;
        let sort_order: i32 = row.get(3)?;
        let bin_id = Uuid::parse_str(&id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?;
        let parent_id = parent_str
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e)))?
            .filter(|pid| *pid != MASTER_BIN_ID);
        Ok(Bin { id: bin_id, name, parent_id, sort_order })
    })?;
    for bin in bin_rows {
        bins.push(bin?);
    }

    let mut source_bins: HashMap<Uuid, Uuid> = HashMap::new();
    let mut bs_stmt = conn.prepare("SELECT bin_id, source_id FROM bin_sources")?;
    let bs_rows = bs_stmt.query_map([], |row| {
        let bin_str: String = row.get(0)?;
        let src_str: String = row.get(1)?;
        let bin_id = Uuid::parse_str(&bin_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?;
        let src_id = Uuid::parse_str(&src_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e)))?;
        Ok((src_id, bin_id))
    })?;
    for row in bs_rows {
        let (src_id, bin_id) = row?;
        if bin_id != MASTER_BIN_ID {
            source_bins.insert(src_id, bin_id);
        }
    }

    // 2b. Load media sources
    let mut sources = HashMap::new();
    let mut source_stmt = conn.prepare(
        "SELECT id, path, relative_path, media_type, duration_us, is_missing,
                video_width, video_height, video_fps_num, video_fps_den, video_codec,
                video_color_range, video_color_space, video_bit_depth, video_has_alpha,
                audio_sample_rate, audio_channels, audio_codec, audio_bit_rate,
                proxy_path, proxy_ready, imported_at, import_order, thumbnail_data,
                display_name, conform_fps_num, conform_fps_den, audio_channel_map
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
        let display_name: Option<String> = row.get(24)?;
        let conform_fps_num: Option<u32> = row.get(25)?;
        let conform_fps_den: Option<u32> = row.get(26)?;
        let audio_channel_map_str: Option<String> = row.get(27)?;

        let media_type = match media_type_int {
            0 => MediaType::Video,
            1 => MediaType::Audio,
            _ => MediaType::Image,
        };

        let video_fps = match (video_fps_num, video_fps_den) {
            (Some(num), Some(den)) => Some(RationalFps { num, den }),
            _ => None,
        };
        let conform_fps = match (conform_fps_num, conform_fps_den) {
            (Some(num), Some(den)) if num > 0 && den > 0 => Some(RationalFps { num, den }),
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
            // Readiness is a cache fact: a proxy file that is gone is rebuilt, not trusted.
            proxy_ready: proxy_ready != 0 && proxy_path_str.as_deref().is_some_and(|p| Path::new(p).is_file()),
            proxy_path: proxy_path_str.map(PathBuf::from),
            thumbnail_data,
            imported_at,
            import_order,
            bin_id: None,
            display_name,
            conform_fps,
            audio_channel_map: audio_channel_map_str.as_deref().map(AudioChannelMap::from_db_str).unwrap_or_default(),
        })
    })?;

    for src in source_rows {
        let mut src = src?;
        src.bin_id = source_bins.get(&src.id).copied();
        sources.insert(src.id, src);
    }

    // 3. Load tracks
    let mut tracks = Vec::new();
    let mut track_stmt = conn.prepare(
        "SELECT id, kind, kind_index, name, enabled, locked, solo, volume, height_px, color, sort_order, timeline_id, audio_type
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

        Ok((Track {
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
            audio_type: if row.get::<_, i32>(12)? == 0 { AudioTrackType::Mono } else { AudioTrackType::Stereo },
        }, row.get::<_, Option<String>>(11)?))
    })?;

    let mut track_owner = Vec::new();
    for trk in track_rows {
        let (track, owner) = trk?;
        tracks.push(track);
        track_owner.push(owner);
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

        let clip = Clip {
            id: Uuid::parse_str(&id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
            track_id: Uuid::parse_str(&track_id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e)))?,
            source_id: Uuid::parse_str(&source_id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e)))?,
            clip_type,
            name,
            timeline_in,
            timeline_out,
            source_in,
            source_out,
            properties: ClipProperties::default(),
            title_data: None,
        };
        Ok((clip, prop_json, title_json))
    })?;

    for row in clip_rows {
        let (mut clip, prop_json, title_json) = row?;
        let corrupt = |field, reason: String| ProjectError::CorruptClip { clip_id: clip.id, field, reason };
        clip.properties = serde_json::from_str(&prop_json).map_err(|e| corrupt("properties", e.to_string()))?;
        clip.title_data = title_json
            .map(|s| serde_json::from_str::<TitleData>(&s))
            .transpose()
            .map_err(|e| corrupt("title", e.to_string()))?;
        let track = tracks
            .iter_mut()
            .find(|t| t.id == clip.track_id)
            .ok_or_else(|| corrupt("track", format!("no track {}", clip.track_id)))?;
        track.clips.push(clip);
    }

    // 5. Load transitions
    let mut trans_stmt = conn.prepare(
        "SELECT id, clip_id, edge, kind, duration_us, alignment, plugin_id, plugin_params, color, ease,
                start_ratio, end_ratio, curve, enabled
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
        let color: Option<String> = row.get(8)?;
        let ease_int: i32 = row.get(9)?;
        let curve: Option<String> = row.get(12)?;
        // A damaged curve costs only the curve, not the project; keys are kept in range and in order.
        let mut curve: Vec<[f32; 2]> = curve
            .and_then(|c| {
                serde_json::from_str(&c).map_err(|e| tracing::warn!(transition = %id_str, error = %e, "unreadable transition curve dropped")).ok()
            })
            .unwrap_or_default();
        curve.retain(|k| k.iter().all(|v| v.is_finite()));
        for k in &mut curve {
            *k = [k[0].clamp(0.0, 1.0), k[1].clamp(0.0, 1.0)];
        }
        curve.sort_by(|a, b| a[0].total_cmp(&b[0]));

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
            9 => TransitionKind::DipToColor,
            10 => TransitionKind::Crossfade,
            11 => TransitionKind::AdditiveDissolve,
            12 => TransitionKind::NonAdditiveDissolve,
            13 => TransitionKind::BlurDissolve,
            _ => TransitionKind::Plugin(plugin_id.clone().unwrap_or_default()),
        };

        let ease = match ease_int {
            1 => TransitionEase::In,
            2 => TransitionEase::Out,
            3 => TransitionEase::InOut,
            _ => TransitionEase::None,
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
            color: color.map(|c| hex_to_color(&c)).transpose().map_err(|e| rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, e.into()))?.unwrap_or_default(),
            ease,
            start_ratio: row.get(10)?,
            end_ratio: row.get(11)?,
            curve,
            enabled: row.get::<_, i32>(13)? != 0,
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
    // Cross dissolves from before schema 4 become transitions.
    for track in &mut tracks {
        track.convert_legacy_dissolves();
    }

    // 6. Load markers
    let mut markers = Vec::new();
    let mut marker_stmt = conn.prepare(
        "SELECT id, position_us, name, color, duration_us, note, timeline_id, keyword
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
            6 => MarkerColor::Cyan,
            7 => MarkerColor::Pink,
            8 => MarkerColor::Fuchsia,
            9 => MarkerColor::Rose,
            10 => MarkerColor::Lavender,
            11 => MarkerColor::Sky,
            12 => MarkerColor::Mint,
            13 => MarkerColor::Lemon,
            14 => MarkerColor::Sand,
            15 => MarkerColor::Cocoa,
            16 => MarkerColor::Cream,
            _ => MarkerColor::Purple,
        };

        Ok((Marker {
            id: Uuid::parse_str(&id_str).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
            position_us,
            name,
            color,
            duration_us,
            note,
            keyword: row.get(7)?,
        }, row.get::<_, Option<String>>(6)?))
    })?;

    let mut marker_owner = Vec::new();
    for m in marker_rows {
        let (marker, owner) = m?;
        markers.push(marker);
        marker_owner.push(owner);
    }
    let (timeline, timelines) = group_timelines(&conn, tracks.into_iter().zip(track_owner), markers.into_iter().zip(marker_owner))?;

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
        timelines,
        bins,
        sources,
        ui_state,
    })
}

/// `[r, g, b]` (0.0-1.0) as `#rrggbb`.
fn color_to_hex(color: [f32; 3]) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(color[0]), byte(color[1]), byte(color[2]))
}

fn hex_to_color(hex: &str) -> std::result::Result<[f32; 3], String> {
    let digits = hex.strip_prefix('#').filter(|d| d.len() == 6 && d.is_ascii()).ok_or_else(|| format!("bad colour {hex:?}"))?;
    let mut color = [0.0f32; 3];
    for (i, c) in color.iter_mut().enumerate() {
        let byte = u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).map_err(|e| format!("bad colour {hex:?}: {e}"))?;
        *c = byte as f32 / 255.0;
    }
    Ok(color)
}

/// Splits loaded tracks and markers into the open timeline and the others, by the
/// timeline each row names. Rows naming none (files from before 0.3.0) go to the open one.
fn group_timelines(
    conn: &rusqlite::Connection,
    tracks: impl Iterator<Item = (Track, Option<String>)>,
    markers: impl Iterator<Item = (Marker, Option<String>)>,
) -> Result<(Timeline, Vec<Timeline>)> {
    let mut stmt = conn.prepare("SELECT id, name, is_open, start_tc_us, width, height, fps_num, fps_den, bin_id FROM timelines ORDER BY rowid")?;
    let rows = stmt.query_map([], |row| {
        let size = (row.get::<_, Option<u32>>(4)?, row.get::<_, Option<u32>>(5)?);
        let resolution = match size {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        };
        let fps = match (row.get::<_, Option<u32>>(6)?, row.get::<_, Option<u32>>(7)?) {
            (Some(num), Some(den)) if num > 0 && den > 0 => Some(RationalFps { num, den }),
            _ => None,
        };
        let bin_id = row
            .get::<_, Option<String>>(8)?
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e)))?
            .filter(|id| *id != MASTER_BIN_ID);
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)? != 0, row.get::<_, i64>(3)?, resolution, fps, bin_id))
    })?;
    let mut all: Vec<(String, Timeline)> = Vec::new();
    let mut open = 0;
    for row in rows {
        let (id, name, is_open, start_tc_us, resolution, fps, bin_id) = row?;
        let uuid = Uuid::parse_str(&id).map_err(|e| ProjectError::InvalidProject(format!("timeline id {id}: {e}")))?;
        if is_open {
            open = all.len();
        }
        all.push((id, Timeline { id: uuid, name, start_tc_us, resolution, fps, bin_id, ..Timeline::new() }));
    }
    if all.is_empty() {
        let t = Timeline::new();
        all.push((t.id.to_string(), t));
    }
    let slot = |owner: &Option<String>| owner.as_ref().and_then(|o| all.iter().position(|(id, _)| id == o)).unwrap_or(open);
    let tracks: Vec<_> = tracks.map(|(t, o)| (slot(&o), t)).collect();
    let markers: Vec<_> = markers.map(|(m, o)| (slot(&o), m)).collect();
    for (i, t) in tracks {
        all[i].1.tracks.push(t);
    }
    for (i, m) in markers {
        all[i].1.markers.push(m);
    }
    let mut all: Vec<Timeline> = all.into_iter().map(|(_, t)| t).collect();
    let timeline = all.remove(open);
    Ok((timeline, all))
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
            color: [0.0; 3],
            ease: TransitionEase::None,
            start_ratio: 0.0,
            end_ratio: 1.0,
            curve: Vec::new(),
            enabled: true,
        };
        project.timeline.tracks[0].transitions.push(trans.clone());

        // Resolve's settings: ratios, curve keys, start timecode, own size, mono track, keyword.
        let trans = Transition { start_ratio: 0.25, end_ratio: 0.75, curve: vec![[0.5, 0.9]], ..trans };
        project.timeline.tracks[0].transitions[0] = trans.clone();
        project.timeline.start_tc_us = tempo_timeline::RESOLVE_START_TC_US;
        project.timeline.resolution = Some((1080, 1920));
        project.timeline.tracks[2].audio_type = AudioTrackType::Mono;
        project.timeline.fps = Some(RationalFps::FPS_29_97);
        project.timeline.tracks[0].transitions[0].enabled = false;

        // Add markers
        let mut m1 = Marker::new(10_000_000, "Scene 1", MarkerColor::Cocoa);
        m1.keyword = "battle".into();
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
        assert_eq!((loaded_trans.start_ratio, loaded_trans.end_ratio, &loaded_trans.curve), (0.25, 0.75, &vec![[0.5, 0.9]]));
        assert_eq!((loaded.timeline.start_tc_us, loaded.timeline.resolution), (tempo_timeline::RESOLVE_START_TC_US, Some((1080, 1920))));
        assert_eq!(loaded.timeline.tracks.iter().map(|t| t.audio_type).collect::<Vec<_>>()[1..3], [AudioTrackType::Stereo, AudioTrackType::Mono]);
        assert_eq!(loaded.timeline.markers[0].keyword, "battle");
        assert_eq!(loaded.timeline.markers[0].color, MarkerColor::Cocoa);
        assert_eq!(loaded.timeline_fps(), RationalFps::FPS_29_97);
        assert!(!loaded.timeline.tracks[0].transitions[0].enabled);
        assert_eq!(loaded.output_size(), (1080, 1920));
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
            color: [0.0; 3],
            ease: TransitionEase::None,
            start_ratio: 0.0,
            end_ratio: 1.0,
            curve: Vec::new(),
            enabled: true,
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
            color: [0.0; 3],
            ease: TransitionEase::None,
            start_ratio: 0.0,
            end_ratio: 1.0,
            curve: Vec::new(),
            enabled: true,
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

    /// A saved one-clip project, then `sql` run on its file without foreign key checks.
    #[test]
    fn several_timelines_keep_their_own_clips_and_markers_and_which_is_open() {
        let file = NamedTempFile::new().unwrap();
        let mut project = Project::new("Two", 1920, 1080, RationalFps::FPS_30);
        let first = project.timeline.id;
        let track_id = project.timeline.tracks[0].id;
        project.timeline.tracks[0].clips.push(Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "c", 0, 1_000_000, 0, 1_000_000));
        project.timeline.add_marker(Marker::new(0, "Section 15", MarkerColor::Blue));
        project.add_timeline(Timeline::with_tracks("Section 15.5 Interlude", 1, 1));
        project.timeline.add_marker(Marker::new(5, "Interlude", MarkerColor::Blue));
        save_project(&project, file.path()).unwrap();

        let loaded = load_project(file.path()).unwrap();
        assert_eq!(loaded.timeline.name, "Section 15.5 Interlude");
        assert_eq!(loaded.timeline.tracks.len(), 2);
        assert_eq!(loaded.timeline.markers.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Interlude"]);
        assert!(loaded.timeline.tracks.iter().all(|t| t.clips.is_empty()));
        assert_eq!(loaded.timelines.len(), 1);
        let other = &loaded.timelines[0];
        assert_eq!((other.id, other.name.as_str()), (first, "Timeline 1"));
        assert_eq!(other.tracks[0].clips.len(), 1);
        assert_eq!(other.markers[0].name, "Section 15");
    }

    #[test]
    fn files_from_before_timelines_load_as_one_timeline() {
        let (file, clip_id) = tampered("DELETE FROM timelines; UPDATE tracks SET timeline_id = NULL; UPDATE markers SET timeline_id = NULL;");
        let loaded = load_project(file.path()).unwrap();
        assert_eq!(loaded.timeline.name, "Timeline 1");
        assert!(loaded.timelines.is_empty());
        assert_eq!(loaded.timeline.tracks.len(), 4);
        assert_eq!(loaded.timeline.tracks[0].clips[0].id, clip_id);
    }

    fn tampered(sql: &str) -> (NamedTempFile, Uuid) {
        let file = NamedTempFile::new().unwrap();
        let mut project = Project::new("Tamper", 1920, 1080, RationalFps::FPS_30);
        let track_id = project.timeline.tracks[0].id;
        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "c", 0, 1_000_000, 0, 1_000_000);
        let clip_id = clip.id;
        project.timeline.tracks[0].clips.push(clip);
        save_project(&project, file.path()).unwrap();
        rusqlite::Connection::open(file.path()).unwrap().execute_batch(sql).unwrap();
        (file, clip_id)
    }

    #[test]
    fn missing_proxy_file_is_not_ready_after_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.tempo");
        let present = dir.path().join("present.mp4");
        std::fs::write(&present, b"x").unwrap();
        let mut project = Project::new("Proxies", 1920, 1080, RationalFps::FPS_30);
        for proxy in [present.clone(), dir.path().join("gone.mp4")] {
            let mut source = MediaSource::new(PathBuf::from("/media/a.mp4"), MediaType::Video, 1_000_000);
            source.proxy_path = Some(proxy);
            source.proxy_ready = true;
            project.sources.insert(source.id, source);
        }
        save_project(&project, &path).unwrap();
        let loaded = load_project(&path).unwrap();
        for source in loaded.sources.values() {
            assert_eq!(source.proxy_ready, source.proxy_path.as_deref() == Some(present.as_path()));
        }
    }

    #[test]
    fn files_without_speed_load_at_normal_speed() {
        let (file, _) = tampered(r#"UPDATE clips SET properties = '{"opacity":1.0}';"#);
        assert_eq!(load_project(file.path()).unwrap().timeline.tracks[0].clips[0].properties.speed, 1.0);
    }

    #[test]
    fn a_copy_saved_under_a_new_id_loads_as_that_project() {
        let file = NamedTempFile::new().unwrap();
        let mut project = Project::new("Original", 1920, 1080, RationalFps::FPS_30);
        save_project(&project, file.path()).unwrap();
        project.id = Uuid::new_v4();
        project.name = "Copy".into();
        save_project(&project, file.path()).unwrap();
        let loaded = load_project(file.path()).unwrap();
        assert_eq!((loaded.id, loaded.name.as_str()), (project.id, "Copy"));
        assert_eq!(crate::read_project_id(file.path()).unwrap(), project.id);
    }

    #[test]
    fn a_retimed_clip_saves_and_loads() {
        // At double speed a clip covers twice as much source as timeline.
        let file = NamedTempFile::new().unwrap();
        let mut project = Project::new("Speed", 1920, 1080, RationalFps::FPS_30);
        let track = project.timeline.tracks[0].id;
        let mut clip = Clip::new(track, Uuid::new_v4(), ClipType::Video, "fast", 0, 2_000_000, 1_000_000, 5_000_000);
        clip.properties.speed = 2.0;
        project.timeline.tracks[0].clips.push(clip.clone());
        save_project(&project, file.path()).expect("save a retimed clip");
        let loaded = load_project(file.path()).unwrap();
        assert_eq!(loaded.timeline.tracks[0].clips[0], clip);
    }

    #[test]
    fn titles_with_missing_fields_still_load() {
        let (file, _) = tampered(r#"UPDATE clips SET title_data = '{"text":"Hello"}';"#);
        let loaded = load_project(file.path()).unwrap();
        assert_eq!(loaded.timeline.tracks[0].clips[0].title_data.as_ref().map(|t| t.text.as_str()), Some("Hello"));
    }

    #[test]
    fn load_rejects_corrupt_clip_properties() {
        let (file, clip_id) = tampered("UPDATE clips SET properties = '{bad';");
        match load_project(file.path()) {
            Err(ProjectError::CorruptClip { clip_id: id, field: "properties", .. }) => assert_eq!(id, clip_id),
            other => panic!("expected CorruptClip, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_corrupt_title_data() {
        let (file, _) = tampered("UPDATE clips SET title_data = '[1,2]';");
        assert!(matches!(load_project(file.path()), Err(ProjectError::CorruptClip { field: "title", .. })));
    }

    #[test]
    fn load_accepts_unknown_and_missing_property_fields() {
        let (file, _) = tampered(r#"UPDATE clips SET properties = '{"opacity":0.5,"future_field":1}';"#);
        let loaded = load_project(file.path()).unwrap();
        let clip = &loaded.timeline.tracks[0].clips[0];
        assert_eq!(clip.properties.opacity, 0.5);
        assert_eq!(clip.properties.volume, 1.0);
    }

    #[test]
    fn load_rejects_clip_on_unknown_track() {
        let (file, _) = tampered(&format!("UPDATE clips SET track_id = '{}';", Uuid::new_v4()));
        assert!(matches!(load_project(file.path()), Err(ProjectError::CorruptClip { field: "track", .. })));
    }

    /// A project with clip A [0, 2 s] and clip B [2, 4 s] (footage from 1 s) on V1.
    fn two_clips() -> (Project, Uuid, Uuid) {
        let mut project = Project::new("Cuts", 1920, 1080, RationalFps::FPS_25);
        let track_id = project.timeline.tracks[0].id;
        let a = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "a", 0, 2_000_000, 0, 2_000_000);
        let b = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "b", 2_000_000, 4_000_000, 1_000_000, 3_000_000);
        let ids = (a.id, b.id);
        project.timeline.tracks[0].clips = vec![a, b];
        (project, ids.0, ids.1)
    }

    #[test]
    fn every_video_dissolve_kind_survives_save_and_load() {
        for kind in TransitionKind::VIDEO_DISSOLVES {
            let file = NamedTempFile::new().unwrap();
            let (mut project, _, b) = two_clips();
            project.timeline.tracks[0].transitions = vec![Transition::new(b, TransitionEdge::In, kind.clone(), 520_000, TransitionAlignment::Centered)];
            save_project(&project, file.path()).unwrap();
            let loaded = load_project(file.path()).unwrap();
            assert_eq!(loaded.timeline.tracks[0].transitions[0].kind, kind);
        }
    }

    #[test]
    fn transitions_keep_kind_colour_and_ease() {
        let file = NamedTempFile::new().unwrap();
        let (mut project, a, b) = two_clips();
        let mut dip = Transition::new(b, TransitionEdge::In, TransitionKind::DipToColor, 520_000, TransitionAlignment::Centered);
        dip.color = [1.0, 0.2, 0.0];
        dip.ease = TransitionEase::InOut;
        let mut fade = Transition::new(a, TransitionEdge::In, TransitionKind::CrossDissolve, 1_000_000, TransitionAlignment::StartAtCut);
        fade.ease = TransitionEase::Out;
        project.timeline.tracks[0].transitions = vec![dip.clone(), fade.clone()];
        save_project(&project, file.path()).unwrap();

        let loaded = load_project(file.path()).unwrap();
        let track = &loaded.timeline.tracks[0];
        let got = track.transition_at_edit(b, TransitionEdge::In).unwrap();
        assert_eq!((got.id, &got.kind, got.duration_us, got.alignment, got.ease), (dip.id, &TransitionKind::DipToColor, 520_000, TransitionAlignment::Centered, TransitionEase::InOut));
        assert!(got.color.iter().zip(dip.color).all(|(x, y)| (x - y).abs() < 0.003), "{:?}", got.color);
        let got = track.transition_at_edit(a, TransitionEdge::In).unwrap();
        assert_eq!((got.id, &got.kind, got.alignment, got.ease, got.color), (fade.id, &fade.kind, fade.alignment, fade.ease, [0.0; 3]));
    }

    #[test]
    fn old_cross_dissolves_load_as_transitions() {
        let file = NamedTempFile::new().unwrap();
        let (mut project, _, b) = two_clips();
        project.timeline.tracks[0].clips[1].properties.dissolve_in_us = 400_000;
        save_project(&project, file.path()).unwrap();

        let loaded = load_project(file.path()).unwrap();
        let track = &loaded.timeline.tracks[0];
        assert_eq!(track.find_clip(b).unwrap().properties.dissolve_in_us, 0);
        let t = track.transition_at_edit(b, TransitionEdge::In).unwrap().clone();
        assert_eq!((t.kind.clone(), t.duration_us, t.alignment), (TransitionKind::CrossDissolve, 400_000, TransitionAlignment::EndAtCut));

        // Saving again keeps the one transition.
        save_project(&loaded, file.path()).unwrap();
        let again = load_project(file.path()).unwrap();
        let ids: Vec<_> = again.timeline.tracks[0].transitions.iter().map(|x| (x.id, x.duration_us)).collect();
        assert_eq!(ids, vec![(t.id, 400_000)]);
    }

    #[test]
    fn schema_3_files_gain_transition_colour_and_ease() {
        let (file, _) = tampered(
            "ALTER TABLE transitions DROP COLUMN color; ALTER TABLE transitions DROP COLUMN ease; DELETE FROM schema_version WHERE version >= 4;
             ALTER TABLE timelines DROP COLUMN start_tc_us; ALTER TABLE timelines DROP COLUMN width; ALTER TABLE timelines DROP COLUMN height;
             ALTER TABLE tracks DROP COLUMN audio_type; ALTER TABLE markers DROP COLUMN keyword;
             ALTER TABLE transitions DROP COLUMN start_ratio; ALTER TABLE transitions DROP COLUMN end_ratio; ALTER TABLE transitions DROP COLUMN curve;
             ALTER TABLE timelines DROP COLUMN fps_num; ALTER TABLE timelines DROP COLUMN fps_den; ALTER TABLE transitions DROP COLUMN enabled;
             ALTER TABLE timelines DROP COLUMN bin_id;
             ALTER TABLE media_sources DROP COLUMN display_name; ALTER TABLE media_sources DROP COLUMN conform_fps_num;
             ALTER TABLE media_sources DROP COLUMN conform_fps_den; ALTER TABLE media_sources DROP COLUMN audio_channel_map;
             INSERT INTO transitions (id, clip_id, edge, kind, duration_us, alignment) SELECT '6f1c3b8e-7d2a-4b8e-9a3c-1d2e3f4a5b6c', id, 1, 1, 500000, 2 FROM clips;",
        );
        let loaded = load_project(file.path()).unwrap();
        let t = &loaded.timeline.tracks[0].transitions[0];
        assert_eq!((t.color, t.ease, t.edge), ([0.0; 3], TransitionEase::None, TransitionEdge::Out));
        // Schema 5 defaults: a plain transition, a stereo track, Start Timecode 0, project size.
        assert_eq!((t.start_ratio, t.end_ratio, t.curve.len()), (0.0, 1.0, 0));
        assert_eq!((loaded.timeline.start_tc_us, loaded.timeline.resolution), (0, None));
        assert!(loaded.timeline.tracks.iter().all(|t| t.audio_type == AudioTrackType::Stereo));
        // Schema 6 defaults: the timeline follows the project's rate and transitions are on.
        assert_eq!(loaded.timeline.fps, None);
        assert!(loaded.timeline.tracks.iter().flat_map(|t| &t.transitions).all(|t| t.enabled));
        // Schema 7 defaults: no user bins, open timeline in Master.
        assert!(loaded.bins.is_empty());
        assert_eq!(loaded.timeline.bin_id, None);
    }

    #[test]
    fn v6_files_upgrade_to_v7_with_bin_id_none() {
        let (file, _) = tampered(
            "ALTER TABLE timelines DROP COLUMN bin_id;
             ALTER TABLE media_sources DROP COLUMN display_name; ALTER TABLE media_sources DROP COLUMN conform_fps_num;
             ALTER TABLE media_sources DROP COLUMN conform_fps_den; ALTER TABLE media_sources DROP COLUMN audio_channel_map;
             DELETE FROM schema_version WHERE version >= 7;",
        );
        let loaded = load_project(file.path()).unwrap();
        assert!(loaded.bins.is_empty());
        assert_eq!(loaded.timeline.bin_id, None);
        let conn = open_connection(file.path()).unwrap();
        let ver: u32 = conn.query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0)).unwrap();
        assert_eq!(ver, crate::CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn bins_sources_and_timelines_round_trip() {
        let file = NamedTempFile::new().unwrap();
        let mut project = Project::new("Bins", 1920, 1080, RationalFps::FPS_30);
        let b_root = project.create_bin("Footage", None).unwrap();
        let b_sub = project.create_bin("Selects", Some(b_root)).unwrap();

        let s_master = MediaSource::new(PathBuf::from("/media/a.mp4"), MediaType::Video, 1_000_000);
        let mut s_sub = MediaSource::new(PathBuf::from("/media/b.mp4"), MediaType::Video, 2_000_000);
        s_sub.import_order = 1;
        let (sm_id, ss_id) = (s_master.id, s_sub.id);
        project.sources.insert(sm_id, s_master);
        project.sources.insert(ss_id, s_sub);
        assert!(project.move_source_to_bin(ss_id, Some(b_sub)));

        let mut t2 = Timeline::with_tracks("Cut B", 1, 1);
        t2.bin_id = Some(b_root);
        let t2_id = t2.id;
        project.timelines.push(t2);
        assert!(project.move_timeline_to_bin(project.timeline.id, Some(b_sub)));

        save_project(&project, file.path()).unwrap();
        let mut loaded = load_project(file.path()).unwrap();
        assert_eq!(loaded.bins.len(), 2);
        assert_eq!(loaded.bins[0].name, "Footage");
        assert_eq!(loaded.bins[0].parent_id, None);
        assert_eq!(loaded.bins[1].name, "Selects");
        assert_eq!(loaded.bins[1].parent_id, Some(b_root));
        assert_eq!(loaded.sources[&sm_id].bin_id, None);
        assert_eq!(loaded.sources[&ss_id].bin_id, Some(b_sub));
        assert_eq!(loaded.timeline.bin_id, Some(b_sub));
        assert_eq!(loaded.timelines.iter().find(|t| t.id == t2_id).unwrap().bin_id, Some(b_root));

        // Verify MASTER_BIN_ID in SQLite maps back to None on load, and deleting a bin persists cleanly.
        assert!(loaded.delete_bin(b_sub));
        save_project(&loaded, file.path()).unwrap();
        let reloaded = load_project(file.path()).unwrap();
        assert_eq!(reloaded.bins.len(), 1);
        assert_eq!(reloaded.sources[&ss_id].bin_id, Some(b_root));
        assert_eq!(reloaded.timeline.bin_id, Some(b_root));
    }

    #[test]
    fn loaded_curve_keys_are_ordered_and_in_range_and_a_damaged_curve_is_dropped() {
        let (file, _) = tampered(
            "INSERT INTO transitions (id, clip_id, edge, kind, duration_us, alignment, curve) SELECT '6f1c3b8e-7d2a-4b8e-9a3c-1d2e3f4a5b6c', id, 1, 1, 500000, 2, '[[0.9, 0.1], [0.2, 2.0]]' FROM clips;",
        );
        let loaded = load_project(file.path()).unwrap();
        assert_eq!(loaded.timeline.tracks[0].transitions[0].curve, vec![[0.2, 1.0], [0.9, 0.1]]);

        let (file, _) = tampered(
            "INSERT INTO transitions (id, clip_id, edge, kind, duration_us, alignment, curve) SELECT '6f1c3b8e-7d2a-4b8e-9a3c-1d2e3f4a5b6c', id, 1, 1, 500000, 2, 'nope' FROM clips;",
        );
        let loaded = load_project(file.path()).expect("a bad curve must not stop the project opening");
        assert!(loaded.timeline.tracks[0].transitions[0].curve.is_empty());
    }

    #[test]
    fn load_rejects_an_unreadable_transition_colour() {
        let (file, _) = tampered(
            "INSERT INTO transitions (id, clip_id, edge, kind, duration_us, alignment, color) SELECT '6f1c3b8e-7d2a-4b8e-9a3c-1d2e3f4a5b6c', id, 1, 9, 500000, 2, 'nope' FROM clips;",
        );
        assert!(load_project(file.path()).is_err());
    }

    #[test]
    fn audio_cross_fades_stay_cross_fades() {
        let file = NamedTempFile::new().unwrap();
        let (mut project, _, b) = two_clips();
        let fade = Transition::new(b, TransitionEdge::In, TransitionKind::Crossfade, 400_000, TransitionAlignment::Centered);
        project.timeline.tracks[0].transitions = vec![fade];
        save_project(&project, file.path()).unwrap();
        let loaded = load_project(file.path()).unwrap();
        assert_eq!(loaded.timeline.tracks[0].transitions[0].kind, TransitionKind::Crossfade);
    }

    #[test]
    fn clip_attributes_round_trip_and_v7_upgrade() {
        let file = NamedTempFile::new().unwrap();
        let mut project = Project::new("Attrs", 1920, 1080, RationalFps::FPS_24);
        let mut src = MediaSource::new(PathBuf::from("/media/take.mp4"), MediaType::Video, 4_000_000);
        src.video_fps = Some(RationalFps::FPS_60);
        src.display_name = Some("Interview Cam A".into());
        src.conform_fps = Some(RationalFps::FPS_24);
        src.audio_channel_map = AudioChannelMap::LeftToBoth;
        let src_id = src.id;
        project.sources.insert(src_id, src);

        let a_track = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
        let mut a_clip = Clip::new(a_track, src_id, ClipType::Audio, "Interview Cam A", 0, 2_000_000, 0, 2_000_000);
        a_clip.properties.audio_channel_map = AudioChannelMap::SwapLeftRight;
        project.timeline.find_track_mut(a_track).unwrap().clips.push(a_clip);

        save_project(&project, file.path()).unwrap();
        let loaded = load_project(file.path()).unwrap();
        let ls = &loaded.sources[&src_id];
        assert_eq!(ls.display_name.as_deref(), Some("Interview Cam A"));
        assert_eq!(ls.conform_fps, Some(RationalFps::FPS_24));
        assert_eq!(ls.audio_channel_map, AudioChannelMap::LeftToBoth);
        assert_eq!(
            loaded.timeline.find_track(a_track).unwrap().clips[0].properties.audio_channel_map,
            AudioChannelMap::SwapLeftRight
        );

        let (v7_file, _) = tampered(
            "ALTER TABLE media_sources DROP COLUMN display_name; ALTER TABLE media_sources DROP COLUMN conform_fps_num;
             ALTER TABLE media_sources DROP COLUMN conform_fps_den; ALTER TABLE media_sources DROP COLUMN audio_channel_map;
             DELETE FROM schema_version WHERE version >= 8;",
        );
        let upgraded = load_project(v7_file.path()).unwrap();
        assert!(upgraded.sources.values().all(|s| s.display_name.is_none() && s.conform_fps.is_none() && s.audio_channel_map == AudioChannelMap::Stereo));
    }
}
