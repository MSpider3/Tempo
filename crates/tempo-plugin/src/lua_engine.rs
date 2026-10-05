use mlua::{Lua, Result as LuaResult};
use std::sync::{Arc, Mutex};
use tempo_timeline::command::{
    AddMarkerCommand, CommandLog, DeleteClipCommand, InsertClipCommand, MoveClipCommand,
    PropertyChange, SetClipPropertyCommand,
};
use tempo_timeline::{Clip, ClipType, Marker, MarkerColor, Timeline, TrackKind};
use uuid::Uuid;

pub struct LuaEngine {
    lua: Lua,
    timeline: Arc<Mutex<Timeline>>,
    command_log: Arc<Mutex<CommandLog>>,
}

impl LuaEngine {
    pub fn new(timeline: Arc<Mutex<Timeline>>, command_log: Arc<Mutex<CommandLog>>) -> LuaResult<Self> {
        let lua = Lua::new();

        let tl_clone = timeline.clone();
        let cmd_clone = command_log.clone();

        let timeline_table = lua.create_table()?;

        // timeline.insert_clip(track, path, src_in, src_out, [timeline_pos])
        let tl_insert = tl_clone.clone();
        let cmd_insert = cmd_clone.clone();
        let insert_fn = lua.create_function(
            move |_, (track_id_or_name, path, src_in, src_out, pos_opt): (String, String, i64, i64, Option<i64>)| {
                let mut guard = tl_insert.lock().unwrap();
                let mut cmd_guard = cmd_insert.lock().unwrap();

                // Find track by UUID or name (e.g. "V1", "A1")
                let track_id = if let Ok(uuid) = Uuid::parse_str(&track_id_or_name) {
                    uuid
                } else {
                    guard
                        .tracks
                        .iter()
                        .find(|t| t.name.eq_ignore_ascii_case(&track_id_or_name))
                        .map(|t| t.id)
                        .unwrap_or_else(|| {
                            if track_id_or_name.starts_with('V') || track_id_or_name.starts_with('v') {
                                guard.tracks.iter().find(|t| t.kind == TrackKind::Video).map(|t| t.id)
                            } else {
                                guard.tracks.iter().find(|t| t.kind == TrackKind::Audio).map(|t| t.id)
                            }
                            .unwrap_or_else(|| guard.tracks[0].id)
                        })
                };

                let timeline_pos = pos_opt.unwrap_or(0);
                let duration = (src_out - src_in).max(0);
                let is_audio = track_id_or_name.starts_with('A') || track_id_or_name.starts_with('a');
                let clip_type = if is_audio { ClipType::Audio } else { ClipType::Video };
                let clip_name = std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Clip".to_string());

                let clip = Clip::new(
                    track_id,
                    Uuid::new_v4(),
                    clip_type,
                    clip_name,
                    timeline_pos,
                    timeline_pos + duration,
                    src_in,
                    src_out,
                );
                let clip_id_str = clip.id.to_string();

                let cmd = Box::new(InsertClipCommand::new(clip));
                let _ = cmd_guard.execute(cmd, &mut guard);

                Ok(clip_id_str)
            },
        )?;
        timeline_table.set("insert_clip", insert_fn)?;

        // timeline.delete_clip(clip_id)
        let tl_del = tl_clone.clone();
        let cmd_del = cmd_clone.clone();
        let del_fn = lua.create_function(move |_, clip_id_str: String| {
            if let Ok(clip_id) = Uuid::parse_str(&clip_id_str) {
                let mut guard = tl_del.lock().unwrap();
                let mut cmd_guard = cmd_del.lock().unwrap();
                let track_id_opt = guard.tracks.iter().find_map(|t| {
                    if t.clips.iter().any(|c| c.id == clip_id) {
                        Some(t.id)
                    } else {
                        None
                    }
                });
                if let Some(track_id) = track_id_opt {
                    let cmd = Box::new(DeleteClipCommand::new(track_id, clip_id));
                    let _ = cmd_guard.execute(cmd, &mut guard);
                }
            }
            Ok(())
        })?;
        timeline_table.set("delete_clip", del_fn)?;

        // timeline.move_clip(clip_id, new_pos)
        let tl_move = tl_clone.clone();
        let cmd_move = cmd_clone.clone();
        let move_fn = lua.create_function(move |_, (clip_id_str, new_pos): (String, i64)| {
            if let Ok(clip_id) = Uuid::parse_str(&clip_id_str) {
                let mut guard = tl_move.lock().unwrap();
                let mut cmd_guard = cmd_move.lock().unwrap();
                let clip_info_opt = guard.tracks.iter().find_map(|t| {
                    t.clips.iter().find(|c| c.id == clip_id).map(|c| (t.id, c.timeline_in))
                });
                if let Some((old_track_id, old_timeline_in)) = clip_info_opt {
                    let cmd = Box::new(MoveClipCommand::new(
                        clip_id,
                        old_track_id,
                        old_track_id,
                        old_timeline_in,
                        new_pos,
                    ));
                    let _ = cmd_guard.execute(cmd, &mut guard);
                }
            }
            Ok(())
        })?;
        timeline_table.set("move_clip", move_fn)?;

        // timeline.set_property(clip_id, key, value)
        let tl_prop = tl_clone.clone();
        let cmd_prop = cmd_clone.clone();
        let prop_fn = lua.create_function(move |_, (clip_id_str, key, val): (String, String, f32)| {
            if let Ok(clip_id) = Uuid::parse_str(&clip_id_str) {
                let mut guard = tl_prop.lock().unwrap();
                let mut cmd_guard = cmd_prop.lock().unwrap();
                let change = match key.to_lowercase().as_str() {
                    "opacity" => PropertyChange::Opacity(val),
                    "volume" => PropertyChange::Volume(val),
                    "rotation" => PropertyChange::Rotation(val),
                    "pan" => PropertyChange::Pan(val),
                    "position_x" => PropertyChange::Position { x: val, y: 0.0 },
                    "position_y" => PropertyChange::Position { x: 0.0, y: val },
                    "scale_x" => PropertyChange::Scale { x: val, y: 1.0 },
                    "scale_y" => PropertyChange::Scale { x: 1.0, y: val },
                    _ => PropertyChange::Opacity(val),
                };
                let cmd = Box::new(SetClipPropertyCommand::new(clip_id, change));
                let _ = cmd_guard.execute(cmd, &mut guard);
            }
            Ok(())
        })?;
        timeline_table.set("set_property", prop_fn)?;

        // timeline.add_marker(time_secs, name, color)
        let tl_marker = tl_clone.clone();
        let cmd_marker = cmd_clone.clone();
        let marker_fn = lua.create_function(
            move |_, (time_secs, name, color_str): (f64, String, String)| {
                let mut guard = tl_marker.lock().unwrap();
                let mut cmd_guard = cmd_marker.lock().unwrap();
                let position_us = (time_secs * 1_000_000.0) as i64;
                let color = match color_str.to_lowercase().as_str() {
                    "blue" => MarkerColor::Blue,
                    "yellow" => MarkerColor::Yellow,
                    "red" => MarkerColor::Red,
                    "purple" => MarkerColor::Purple,
                    "orange" => MarkerColor::Orange,
                    _ => MarkerColor::Green,
                };
                let marker = Marker::new(position_us, name, color);
                let cmd = Box::new(AddMarkerCommand::new(marker));
                let _ = cmd_guard.execute(cmd, &mut guard);
                Ok(())
            },
        )?;
        timeline_table.set("add_marker", marker_fn)?;

        lua.globals().set("timeline", timeline_table)?;

        // tempo table
        let tempo_table = lua.create_table()?;
        let notify_fn = lua.create_function(|_, msg: String| {
            tracing::info!("[Lua tempo.notify] {}", msg);
            Ok(())
        })?;
        tempo_table.set("notify", notify_fn)?;
        lua.globals().set("tempo", tempo_table)?;

        Ok(Self {
            lua,
            timeline,
            command_log,
        })
    }

    pub fn execute_script(&self, script: &str) -> LuaResult<()> {
        self.lua.load(script).exec()
    }

    pub fn timeline(&self) -> Arc<Mutex<Timeline>> {
        self.timeline.clone()
    }

    pub fn command_log(&self) -> Arc<Mutex<CommandLog>> {
        self.command_log.clone()
    }
}
