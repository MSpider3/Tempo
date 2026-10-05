use crate::memory::{read_wasm_string, write_wasm_bytes, WasmClipInfo, WasmMediaInfo};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tempo_timeline::command::{
    CommandLog, DeleteClipCommand, InsertClipCommand, MoveClipCommand, PropertyChange,
    SetClipPropertyCommand,
};
use tempo_timeline::{Clip, ClipType, Timeline, TrackKind};
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use wasmtime::{Caller, Linker, StoreLimits, StoreLimitsBuilder};

pub struct HostState {
    pub timeline: Option<Arc<Mutex<Timeline>>>,
    pub command_log: Option<Arc<Mutex<CommandLog>>>,
    pub playhead_us: i64,
    pub registered_effects: Arc<Mutex<HashMap<String, String>>>,
    pub logs: Arc<Mutex<Vec<(i32, String)>>>,
    pub notifications: Arc<Mutex<Vec<String>>>,
    pub limits: StoreLimits,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            timeline: None,
            command_log: None,
            playhead_us: 0,
            registered_effects: Arc::new(Mutex::new(HashMap::new())),
            logs: Arc::new(Mutex::new(Vec::new())),
            notifications: Arc::new(Mutex::new(Vec::new())),
            // Limit WASM instance to 256 MB per PLUGIN_SPEC.md §4.3
            limits: StoreLimitsBuilder::new()
                .memory_size(256 * 1024 * 1024)
                .build(),
        }
    }
}

pub fn register_host_functions(linker: &mut Linker<HostState>) -> Result<(), wasmtime::Error> {
    // 1. tempo::log(level, msg_ptr, msg_len)
    linker.func_wrap(
        "tempo",
        "log",
        |mut caller: Caller<'_, HostState>, level: i32, msg_ptr: i32, msg_len: i32| {
            if let Ok(msg) = read_wasm_string(&mut caller, msg_ptr, msg_len) {
                match level {
                    0 => debug!(target: "tempo_plugin", "[Plugin Debug] {}", msg),
                    1 => info!(target: "tempo_plugin", "[Plugin Info] {}", msg),
                    2 => warn!(target: "tempo_plugin", "[Plugin Warn] {}", msg),
                    _ => error!(target: "tempo_plugin", "[Plugin Error] {}", msg),
                }
                caller.data().logs.lock().unwrap().push((level, msg));
            }
        },
    )?;

    // 2. tempo::timeline_get_track_count() -> i32
    linker.func_wrap("tempo", "timeline_get_track_count", |caller: Caller<'_, HostState>| -> i32 {
        if let Some(ref tl) = caller.data().timeline {
            tl.lock().unwrap().tracks.len() as i32
        } else {
            0
        }
    })?;

    // 3. tempo::timeline_get_track_id(index, out_ptr)
    linker.func_wrap(
        "tempo",
        "timeline_get_track_id",
        |mut caller: Caller<'_, HostState>, index: i32, out_ptr: i32| {
            let track_id = if let Some(ref tl) = caller.data().timeline {
                let guard = tl.lock().unwrap();
                if (index as usize) < guard.tracks.len() {
                    Some(guard.tracks[index as usize].id)
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(id) = track_id {
                let bytes = id.as_bytes();
                let _ = write_wasm_bytes(&mut caller, out_ptr, bytes);
            }
        },
    )?;

    // 4. tempo::timeline_get_clip_count(track_id_ptr) -> i32
    linker.func_wrap(
        "tempo",
        "timeline_get_clip_count",
        |mut caller: Caller<'_, HostState>, track_id_ptr: i32| -> i32 {
            let mut id_bytes = [0u8; 16];
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(m)) => m,
                _ => return 0,
            };
            let data = memory.data(&caller);
            let start = track_id_ptr as usize;
            if start + 16 > data.len() {
                return 0;
            }
            id_bytes.copy_from_slice(&data[start..start + 16]);
            let track_id = Uuid::from_bytes(id_bytes);

            if let Some(ref tl) = caller.data().timeline {
                let guard = tl.lock().unwrap();
                if let Some(track) = guard.tracks.iter().find(|t| t.id == track_id) {
                    return track.clips.len() as i32;
                }
            }
            0
        },
    )?;

    // 5. tempo::timeline_get_clip(track_id_ptr, index, out_ptr) -> i32
    linker.func_wrap(
        "tempo",
        "timeline_get_clip",
        |mut caller: Caller<'_, HostState>, track_id_ptr: i32, index: i32, out_ptr: i32| -> i32 {
            let mut id_bytes = [0u8; 16];
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(m)) => m,
                _ => return -1,
            };
            let data = memory.data(&caller);
            let start = track_id_ptr as usize;
            if start + 16 > data.len() {
                return -1;
            }
            id_bytes.copy_from_slice(&data[start..start + 16]);
            let track_id = Uuid::from_bytes(id_bytes);

            let clip_info = if let Some(ref tl) = caller.data().timeline {
                let guard = tl.lock().unwrap();
                if let Some(track) = guard.tracks.iter().find(|t| t.id == track_id) {
                    if (index as usize) < track.clips.len() {
                        let clip = &track.clips[index as usize];
                        Some(WasmClipInfo {
                            clip_id: *clip.id.as_bytes(),
                            source_id: *clip.source_id.as_bytes(),
                            timeline_in_us: clip.timeline_in,
                            timeline_out_us: clip.timeline_out,
                            source_in_us: clip.source_in,
                            source_out_us: clip.source_out,
                            clip_type: if track.kind == TrackKind::Video { 0 } else { 1 },
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(info) = clip_info {
                let bytes = info.to_bytes();
                if write_wasm_bytes(&mut caller, out_ptr, &bytes).is_ok() {
                    return 0;
                }
            }
            -1
        },
    )?;

    // 6. tempo::timeline_get_duration_us() -> i64
    linker.func_wrap("tempo", "timeline_get_duration_us", |caller: Caller<'_, HostState>| -> i64 {
        if let Some(ref tl) = caller.data().timeline {
            tl.lock().unwrap().duration_us()
        } else {
            0
        }
    })?;

    // 7. tempo::timeline_get_playhead_us() -> i64
    linker.func_wrap("tempo", "timeline_get_playhead_us", |caller: Caller<'_, HostState>| -> i64 {
        caller.data().playhead_us
    })?;

    // 8. tempo::timeline_insert_clip(track_id_ptr, path_ptr, path_len, src_in, src_out, timeline_pos) -> i32
    linker.func_wrap(
        "tempo",
        "timeline_insert_clip",
        |mut caller: Caller<'_, HostState>,
         track_id_ptr: i32,
         path_ptr: i32,
         path_len: i32,
         src_in: i64,
         src_out: i64,
         timeline_pos: i64|
         -> i32 {
            let mut id_bytes = [0u8; 16];
            {
                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };
                let data = memory.data(&caller);
                let start = track_id_ptr as usize;
                if start + 16 > data.len() {
                    return -1;
                }
                id_bytes.copy_from_slice(&data[start..start + 16]);
            }
            let track_id = Uuid::from_bytes(id_bytes);
            let path = match read_wasm_string(&mut caller, path_ptr, path_len) {
                Ok(p) => p,
                Err(_) => return -1,
            };

            let duration = (src_out - src_in).max(0);
            let clip = Clip::new(
                track_id,
                Uuid::new_v4(),
                ClipType::Video,
                path,
                timeline_pos,
                timeline_pos + duration,
                src_in,
                src_out,
            );

            if let (Some(ref tl), Some(ref cmd_log)) = (&caller.data().timeline, &caller.data().command_log) {
                let mut guard = tl.lock().unwrap();
                let mut cmd_guard = cmd_log.lock().unwrap();
                let cmd = Box::new(InsertClipCommand::new(clip));
                if cmd_guard.execute(cmd, &mut guard).is_ok() {
                    return 0;
                }
            } else if let Some(ref tl) = caller.data().timeline {
                let mut guard = tl.lock().unwrap();
                if let Some(track) = guard.tracks.iter_mut().find(|t| t.id == track_id) {
                    let idx = track.clips.len() as i32;
                    track.clips.push(clip);
                    return idx;
                }
            }
            -1
        },
    )?;

    // 9. tempo::timeline_delete_clip(clip_id_ptr) -> i32
    linker.func_wrap(
        "tempo",
        "timeline_delete_clip",
        |mut caller: Caller<'_, HostState>, clip_id_ptr: i32| -> i32 {
            let mut id_bytes = [0u8; 16];
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(m)) => m,
                _ => return -1,
            };
            let data = memory.data(&caller);
            let start = clip_id_ptr as usize;
            if start + 16 > data.len() {
                return -1;
            }
            id_bytes.copy_from_slice(&data[start..start + 16]);
            let clip_id = Uuid::from_bytes(id_bytes);

            if let (Some(ref tl), Some(ref cmd_log)) = (&caller.data().timeline, &caller.data().command_log) {
                let mut guard = tl.lock().unwrap();
                let mut cmd_guard = cmd_log.lock().unwrap();

                // Find track containing clip
                let track_id_opt = guard.tracks.iter().find_map(|t| {
                    if t.clips.iter().any(|c| c.id == clip_id) {
                        Some(t.id)
                    } else {
                        None
                    }
                });

                if let Some(track_id) = track_id_opt {
                    let cmd = Box::new(DeleteClipCommand::new(track_id, clip_id));
                    if cmd_guard.execute(cmd, &mut guard).is_ok() {
                        return 0;
                    }
                }
            } else if let Some(ref tl) = caller.data().timeline {
                let mut guard = tl.lock().unwrap();
                for track in &mut guard.tracks {
                    if let Some(pos) = track.clips.iter().position(|c| c.id == clip_id) {
                        track.clips.remove(pos);
                        return 0;
                    }
                }
            }
            -1
        },
    )?;

    // 10. tempo::timeline_move_clip(clip_id_ptr, new_in, new_track_id_ptr) -> i32
    linker.func_wrap(
        "tempo",
        "timeline_move_clip",
        |mut caller: Caller<'_, HostState>,
         clip_id_ptr: i32,
         new_in: i64,
         new_track_id_ptr: i32|
         -> i32 {
            let mut clip_id_bytes = [0u8; 16];
            let mut track_id_bytes = [0u8; 16];
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(m)) => m,
                _ => return -1,
            };
            let data = memory.data(&caller);
            let start = clip_id_ptr as usize;
            if start + 16 > data.len() {
                return -1;
            }
            clip_id_bytes.copy_from_slice(&data[start..start + 16]);
            let clip_id = Uuid::from_bytes(clip_id_bytes);

            let new_track_id = if new_track_id_ptr >= 0 && (new_track_id_ptr as usize + 16) <= data.len() {
                let t_start = new_track_id_ptr as usize;
                track_id_bytes.copy_from_slice(&data[t_start..t_start + 16]);
                Some(Uuid::from_bytes(track_id_bytes))
            } else {
                None
            };

            if let (Some(ref tl), Some(ref cmd_log)) = (&caller.data().timeline, &caller.data().command_log) {
                let mut guard = tl.lock().unwrap();
                let mut cmd_guard = cmd_log.lock().unwrap();

                // Find current track and clip info
                let clip_info_opt = guard.tracks.iter().find_map(|t| {
                    t.clips.iter().find(|c| c.id == clip_id).map(|c| (t.id, c.timeline_in))
                });

                if let Some((old_track_id, old_timeline_in)) = clip_info_opt {
                    let target_track_id = new_track_id.unwrap_or(old_track_id);
                    let cmd = Box::new(MoveClipCommand::new(
                        clip_id,
                        old_track_id,
                        target_track_id,
                        old_timeline_in,
                        new_in,
                    ));
                    if cmd_guard.execute(cmd, &mut guard).is_ok() {
                        return 0;
                    }
                }
            }
            -1
        },
    )?;

    // 11. tempo::timeline_set_clip_property(clip_id_ptr, unused, key_ptr, key_len, val_ptr, val_len) -> i32
    linker.func_wrap(
        "tempo",
        "timeline_set_clip_property",
        |mut caller: Caller<'_, HostState>,
         clip_id_ptr: i32,
         _unused: i32,
         key_ptr: i32,
         key_len: i32,
         val_ptr: i32,
         val_len: i32|
         -> i32 {
            let mut clip_id_bytes = [0u8; 16];
            {
                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };
                let data = memory.data(&caller);
                let start = clip_id_ptr as usize;
                if start + 16 > data.len() {
                    return -1;
                }
                clip_id_bytes.copy_from_slice(&data[start..start + 16]);
            }
            let clip_id = Uuid::from_bytes(clip_id_bytes);
            let key = match read_wasm_string(&mut caller, key_ptr, key_len) {
                Ok(k) => k,
                Err(_) => return -1,
            };
            let val = match read_wasm_string(&mut caller, val_ptr, val_len) {
                Ok(v) => v,
                Err(_) => return -2,
            };

            let float_val = match val.parse::<f32>() {
                Ok(f) => f,
                Err(_) => match serde_json::from_str::<f32>(&val) {
                    Ok(f) => f,
                    Err(_) => return -2,
                },
            };

            let change = match key.to_lowercase().as_str() {
                "opacity" => PropertyChange::Opacity(float_val),
                "volume" => PropertyChange::Volume(float_val),
                "rotation" => PropertyChange::Rotation(float_val),
                "pan" => PropertyChange::Pan(float_val),
                "position_x" => PropertyChange::Position { x: float_val, y: 0.0 },
                "position_y" => PropertyChange::Position { x: 0.0, y: float_val },
                "scale_x" => PropertyChange::Scale { x: float_val, y: 1.0 },
                "scale_y" => PropertyChange::Scale { x: 1.0, y: float_val },
                _ => PropertyChange::Opacity(float_val),
            };

            if let (Some(ref tl), Some(ref cmd_log)) = (&caller.data().timeline, &caller.data().command_log) {
                let mut guard = tl.lock().unwrap();
                let mut cmd_guard = cmd_log.lock().unwrap();
                let cmd = Box::new(SetClipPropertyCommand::new(clip_id, change));
                if cmd_guard.execute(cmd, &mut guard).is_ok() {
                    return 0;
                }
            }
            -1
        },
    )?;

    // 12. tempo::render_register_effect(id_ptr, id_len, wgsl_ptr, wgsl_len) -> i32
    linker.func_wrap(
        "tempo",
        "render_register_effect",
        |mut caller: Caller<'_, HostState>, id_ptr: i32, id_len: i32, wgsl_ptr: i32, wgsl_len: i32| -> i32 {
            let id = match read_wasm_string(&mut caller, id_ptr, id_len) {
                Ok(s) => s,
                Err(_) => return -1,
            };
            let wgsl = match read_wasm_string(&mut caller, wgsl_ptr, wgsl_len) {
                Ok(s) => s,
                Err(_) => return -1,
            };
            caller.data().registered_effects.lock().unwrap().insert(id, wgsl);
            0
        },
    )?;

    // 13. tempo::render_unregister_effect(id_ptr, id_len)
    linker.func_wrap(
        "tempo",
        "render_unregister_effect",
        |mut caller: Caller<'_, HostState>, id_ptr: i32, id_len: i32| {
            if let Ok(id) = read_wasm_string(&mut caller, id_ptr, id_len) {
                caller.data().registered_effects.lock().unwrap().remove(&id);
            }
        },
    )?;

    // 14. tempo::ui_add_panel(panel_id_ptr, panel_id_len, title_ptr, title_len, position) -> i32
    linker.func_wrap(
        "tempo",
        "ui_add_panel",
        |_caller: Caller<'_, HostState>,
         _panel_id_ptr: i32,
         _panel_id_len: i32,
         _title_ptr: i32,
         _title_len: i32,
         _position: i32|
         -> i32 {
            0
        },
    )?;

    // 15. tempo::ui_remove_panel(panel_id_ptr, panel_id_len)
    linker.func_wrap(
        "tempo",
        "ui_remove_panel",
        |_caller: Caller<'_, HostState>, _panel_id_ptr: i32, _panel_id_len: i32| {},
    )?;

    // 16. tempo::ui_send_html(panel_id_ptr, panel_id_len, html_ptr, html_len)
    linker.func_wrap(
        "tempo",
        "ui_send_html",
        |_caller: Caller<'_, HostState>, _panel_id_ptr: i32, _panel_id_len: i32, _html_ptr: i32, _html_len: i32| {},
    )?;

    // 17. tempo::ui_execute_js(panel_id_ptr, panel_id_len, js_ptr, js_len)
    linker.func_wrap(
        "tempo",
        "ui_execute_js",
        |_caller: Caller<'_, HostState>, _panel_id_ptr: i32, _panel_id_len: i32, _js_ptr: i32, _js_len: i32| {},
    )?;

    // 18. tempo::ui_notify(msg_ptr, msg_len, timeout_ms)
    linker.func_wrap(
        "tempo",
        "ui_notify",
        |mut caller: Caller<'_, HostState>, msg_ptr: i32, msg_len: i32, _timeout_ms: i32| {
            if let Ok(msg) = read_wasm_string(&mut caller, msg_ptr, msg_len) {
                caller.data().notifications.lock().unwrap().push(msg);
            }
        },
    )?;

    // 19. tempo::media_probe(path_ptr, path_len, out_ptr) -> i32
    linker.func_wrap(
        "tempo",
        "media_probe",
        |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32, out_ptr: i32| -> i32 {
            let path = match read_wasm_string(&mut caller, path_ptr, path_len) {
                Ok(p) => p,
                Err(_) => return -1,
            };
            if !std::path::Path::new(&path).exists() {
                return -1;
            }
            let info = WasmMediaInfo {
                struct_size: 40,
                media_type: 0,
                duration_us: 10_000_000,
                width: 1920,
                height: 1080,
                fps_num: 30,
                fps_den: 1,
                sample_rate: 48000,
                channels: 2,
            };
            let bytes = info.to_bytes();
            if write_wasm_bytes(&mut caller, out_ptr, &bytes).is_ok() {
                0
            } else {
                -2
            }
        },
    )?;

    // 20. tempo::compute_call(method_ptr, method_len, params_ptr, params_len, out_ptr, out_max) -> i32
    linker.func_wrap(
        "tempo",
        "compute_call",
        |_caller: Caller<'_, HostState>,
         _method_ptr: i32,
         _method_len: i32,
         _params_ptr: i32,
         _params_len: i32,
         _out_ptr: i32,
         _out_max: i32|
         -> i32 {
            -1
        },
    )?;

    // 21. tempo::alloc(size) -> i32
    linker.func_wrap("tempo", "alloc", |_caller: Caller<'_, HostState>, _size: i32| -> i32 {
        0
    })?;

    // 22. tempo::free(ptr)
    linker.func_wrap("tempo", "free", |_caller: Caller<'_, HostState>, _ptr: i32| {})?;

    Ok(())
}
