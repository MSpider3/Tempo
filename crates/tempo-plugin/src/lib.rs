//! tempo-plugin: Lua script plugins.
//!
//! A plugin is a folder with a `plugin.toml` manifest and a `main.lua` script.
//! Each command it declares names a Lua function. A command runs in a fresh,
//! sandboxed Lua state against a *copy* of the timeline; the caller gets the
//! edited copy back and applies it as a single undo step. A script therefore
//! cannot freeze the editor, touch files, or leave the timeline half changed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use serde::Deserialize;
use tempo_timeline::{
    AddMarkerCommand, Clip, ClipType, Command, DeleteClipCommand, DeleteMarkerCommand, Marker, MarkerColor, MoveClipCommand,
    RippleDeleteCommand, SplitClipCommand, Timeline, TrackKind,
};
use uuid::Uuid;

/// A script may use this much memory and run for this long.
pub const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
pub const TIME_LIMIT: Duration = Duration::from_secs(5);
/// Loudness data has this many values per second of sound.
pub const PEAKS_PER_SECOND: f64 = 50.0;

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("cannot read {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("plugin.toml is not valid: {0}")]
    Manifest(String),
    #[error("{0}")]
    Script(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TimelineAccess {
    #[default]
    None,
    Read,
    Write,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandDef {
    pub id: String,
    pub name: String,
    pub function: String,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    plugin: ManifestHead,
    #[serde(default)]
    permissions: Permissions,
    #[serde(default, rename = "command")]
    commands: Vec<CommandDef>,
}

#[derive(Debug, Deserialize)]
struct ManifestHead {
    id: String,
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize, Default)]
struct Permissions {
    #[serde(default)]
    timeline: TimelineAccess,
}

/// A plugin that has been read and checked, ready to run.
#[derive(Debug, Clone)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub timeline: TimelineAccess,
    pub commands: Vec<CommandDef>,
    /// The Lua source.
    pub script: Arc<str>,
    /// True for plugins that ship inside the program.
    pub built_in: bool,
}

impl Plugin {
    /// Build a plugin from the text of its two files.
    pub fn from_sources(manifest: &str, script: &str, built_in: bool) -> Result<Self, PluginError> {
        let m: ManifestFile = toml::from_str(manifest).map_err(|e| PluginError::Manifest(e.to_string()))?;
        if m.plugin.id.trim().is_empty() || m.plugin.name.trim().is_empty() {
            return Err(PluginError::Manifest("the plugin needs an id and a name".into()));
        }
        Ok(Self {
            id: m.plugin.id,
            name: m.plugin.name,
            version: m.plugin.version,
            description: m.plugin.description,
            timeline: m.permissions.timeline,
            commands: m.commands,
            script: script.into(),
            built_in,
        })
    }

    /// Read a plugin folder. Blocking.
    pub fn load(dir: &Path) -> Result<Self, PluginError> {
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read_to_string(&path).map_err(|e| PluginError::Io(path, e))
        };
        Self::from_sources(&read("plugin.toml")?, &read("main.lua")?, false)
    }
}

/// The plugins that ship with Tempo.
pub fn built_in() -> Vec<Plugin> {
    let sources = [(
        include_str!("../../../assets/plugins/remove-silence/plugin.toml"),
        include_str!("../../../assets/plugins/remove-silence/main.lua"),
    )];
    sources
        .iter()
        .filter_map(|(manifest, script)| match Plugin::from_sources(manifest, script, true) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::error!("built-in plugin is broken: {e}");
                None
            }
        })
        .collect()
}

/// Every plugin folder under `dir` that loads. Blocking. Broken ones are logged and skipped.
pub fn discover(dir: &Path) -> Vec<Plugin> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut plugins: Vec<Plugin> = entries
        .flatten()
        .filter(|e| e.path().join("plugin.toml").exists())
        .filter_map(|e| match Plugin::load(&e.path()) {
            Ok(p) => Some(p),
            Err(err) => {
                tracing::warn!("skipping plugin in {}: {err}", e.path().display());
                None
            }
        })
        .collect();
    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    plugins
}

/// What a command sees when it starts.
pub struct RunInput {
    pub timeline: Timeline,
    pub playhead_us: i64,
    pub selection: Vec<Uuid>,
    /// Loudness of each media source (0–255, `PEAKS_PER_SECOND` values a second).
    pub loudness: HashMap<Uuid, Arc<Vec<u8>>>,
}

/// What a command leaves behind.
pub struct RunOutput {
    /// The timeline after the script; apply it as one undo step if `changed`.
    pub timeline: Timeline,
    pub changed: bool,
    /// Messages from `tempo.notify`, in order.
    pub messages: Vec<String>,
}

struct Session {
    timeline: Timeline,
    can_write: bool,
    can_read: bool,
    messages: Vec<String>,
}

fn script_error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

fn parse_id(text: &str) -> mlua::Result<Uuid> {
    Uuid::parse_str(text).map_err(|_| script_error(format!("'{text}' is not a clip, track or marker id")))
}

fn us(seconds: f64) -> i64 {
    (seconds * 1_000_000.0).round() as i64
}

fn secs(us: i64) -> f64 {
    us as f64 / 1_000_000.0
}

fn clip_table(lua: &Lua, clip: &Clip) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("id", clip.id.to_string())?;
    t.set("name", clip.name.as_str())?;
    t.set("start", secs(clip.timeline_in))?;
    t.set("duration", secs(clip.duration_us()))?;
    t.set("source_start", secs(clip.source_in))?;
    t.set(
        "kind",
        match clip.clip_type {
            ClipType::Video => "video",
            ClipType::Audio => "audio",
            ClipType::Image => "image",
            ClipType::Title => "title",
        },
    )?;
    t.set("enabled", clip.properties.enabled)?;
    t.set("volume", clip.properties.volume)?;
    Ok(t)
}

/// Run one command of a plugin. Blocking; call from a worker thread.
pub fn run_command(plugin: &Plugin, function: &str, input: RunInput) -> Result<RunOutput, PluginError> {
    let fail = |e: mlua::Error| PluginError::Script(first_line(&e.to_string()));
    let before = input.timeline.clone();
    let session = Rc::new(RefCell::new(Session {
        timeline: input.timeline,
        can_write: plugin.timeline == TimelineAccess::Write,
        can_read: plugin.timeline != TimelineAccess::None,
        messages: Vec::new(),
    }));

    // Only the pure libraries: no files, no processes, no loading of other code.
    let lua = Lua::new_with(StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8, LuaOptions::default()).map_err(fail)?;
    lua.set_memory_limit(MEMORY_LIMIT).map_err(fail)?;
    let deadline = Instant::now() + TIME_LIMIT;
    lua.set_hook(HookTriggers::new().every_nth_instruction(10_000), move |_, _| {
        if Instant::now() > deadline {
            Err(script_error("the script ran for too long and was stopped"))
        } else {
            Ok(VmState::Continue)
        }
    });
    let globals = lua.globals();
    for name in ["dofile", "loadfile", "load", "loadstring", "require", "collectgarbage"] {
        globals.set(name, Value::Nil).map_err(fail)?;
    }

    install_api(&lua, &session, input.playhead_us, &input.selection, input.loudness).map_err(fail)?;

    lua.load(&*plugin.script).set_name(plugin.name.as_str()).exec().map_err(fail)?;
    let entry: mlua::Function = globals
        .get(function)
        .map_err(|_| PluginError::Script(format!("the plugin has no function called '{function}'")))?;
    entry.call::<()>(()).map_err(fail)?;
    drop(lua);

    let session = Rc::try_unwrap(session).map_err(|_| PluginError::Script("the script kept a reference to the timeline".into()))?.into_inner();
    Ok(RunOutput { changed: session.timeline != before, timeline: session.timeline, messages: session.messages })
}

fn first_line(text: &str) -> String {
    text.lines().find(|l| !l.trim().is_empty()).unwrap_or("the script failed").trim().to_string()
}

fn install_api(lua: &Lua, session: &Rc<RefCell<Session>>, playhead_us: i64, selection: &[Uuid], loudness: HashMap<Uuid, Arc<Vec<u8>>>) -> mlua::Result<()> {
    let tempo = lua.create_table()?;

    // ---- Always available -----------------------------------------------------
    let s = session.clone();
    let notify = lua.create_function(move |_, text: String| {
        s.borrow_mut().messages.push(text);
        Ok(())
    })?;
    tempo.set("notify", notify)?;
    tempo.set("log", lua.create_function(|_, text: String| {
        tracing::info!("plugin: {text}");
        Ok(())
    })?)?;
    lua.globals().set("print", lua.create_function(|_, text: String| {
        tracing::info!("plugin: {text}");
        Ok(())
    })?)?;
    tempo.set("playhead", lua.create_function(move |_, ()| Ok(secs(playhead_us)))?)?;
    let ids: Vec<String> = selection.iter().map(Uuid::to_string).collect();
    tempo.set("selection", lua.create_function(move |_, ()| Ok(ids.clone()))?)?;

    // Guards used by the functions below.
    fn need_read(s: &Session) -> mlua::Result<()> {
        if s.can_read { Ok(()) } else { Err(script_error("this plugin has no permission to read the timeline")) }
    }
    fn need_write(s: &Session) -> mlua::Result<()> {
        if s.can_write { Ok(()) } else { Err(script_error("this plugin has no permission to change the timeline")) }
    }
    fn run(s: &mut Session, mut command: impl Command) -> mlua::Result<()> {
        command.execute(&mut s.timeline).map_err(|e| script_error(e.to_string()))
    }

    // ---- Timeline: read ---------------------------------------------------------
    let timeline = lua.create_table()?;
    let s = session.clone();
    timeline.set("tracks", lua.create_function(move |lua, ()| {
        let s = s.borrow();
        need_read(&s)?;
        let list = lua.create_table()?;
        for track in &s.timeline.tracks {
            let t = lua.create_table()?;
            t.set("id", track.id.to_string())?;
            t.set("kind", if track.kind == TrackKind::Video { "video" } else { "audio" })?;
            t.set("index", track.kind_index)?;
            t.set("name", track.name.as_str())?;
            t.set("locked", track.locked)?;
            list.push(t)?;
        }
        Ok(list)
    })?)?;
    let s = session.clone();
    timeline.set("clips", lua.create_function(move |lua, track: String| {
        let s = s.borrow();
        need_read(&s)?;
        let track = s.timeline.find_track(parse_id(&track)?).ok_or_else(|| script_error("no such track"))?;
        let list = lua.create_table()?;
        for clip in &track.clips {
            list.push(clip_table(lua, clip)?)?;
        }
        Ok(list)
    })?)?;
    let s = session.clone();
    timeline.set("clip", lua.create_function(move |lua, id: String| {
        let s = s.borrow();
        need_read(&s)?;
        match s.timeline.find_clip(parse_id(&id)?) {
            Some((_, clip)) => Ok(Value::Table(clip_table(lua, clip)?)),
            None => Ok(Value::Nil),
        }
    })?)?;

    // ---- Timeline: write ----------------------------------------------------------
    let s = session.clone();
    timeline.set("split", lua.create_function(move |_, (id, at): (String, f64)| {
        let mut s = s.borrow_mut();
        need_write(&s)?;
        let id = parse_id(&id)?;
        let track = s.timeline.find_clip(id).map(|(t, _)| t.id).ok_or_else(|| script_error("no such clip"))?;
        let mut command = SplitClipCommand::new(track, id, us(at));
        command.execute(&mut s.timeline).map_err(|e| script_error(e.to_string()))?;
        // The part after the cut gets a new id; the part before keeps the old one.
        Ok(command.second_clip_id().to_string())
    })?)?;
    let s = session.clone();
    timeline.set("delete", lua.create_function(move |_, (id, options): (String, Option<Table>)| {
        let mut s = s.borrow_mut();
        need_write(&s)?;
        let id = parse_id(&id)?;
        let track = s.timeline.find_clip(id).map(|(t, _)| t.id).ok_or_else(|| script_error("no such clip"))?;
        let ripple = options.and_then(|o| o.get::<bool>("ripple").ok()).unwrap_or(false);
        if ripple {
            run(&mut s, RippleDeleteCommand::new(track, id))
        } else {
            run(&mut s, DeleteClipCommand::new(track, id))
        }
    })?)?;
    let s = session.clone();
    timeline.set("move", lua.create_function(move |_, (id, start): (String, f64)| {
        let mut s = s.borrow_mut();
        need_write(&s)?;
        let id = parse_id(&id)?;
        let (track, from) = s.timeline.find_clip(id).map(|(t, c)| (t.id, c.timeline_in)).ok_or_else(|| script_error("no such clip"))?;
        run(&mut s, MoveClipCommand::new(id, track, track, from, us(start).max(0)))
    })?)?;
    let s = session.clone();
    timeline.set("set", lua.create_function(move |_, (id, property, value): (String, String, Value)| {
        let mut s = s.borrow_mut();
        need_write(&s)?;
        let clip = s.timeline.find_clip_mut(parse_id(&id)?).ok_or_else(|| script_error("no such clip"))?;
        let number = || match &value {
            Value::Number(n) => Ok(*n),
            Value::Integer(n) => Ok(*n as f64),
            _ => Err(script_error(format!("'{property}' needs a number"))),
        };
        let half = clip.duration_us() / 2;
        match property.as_str() {
            "volume" => clip.properties.volume = number()?.clamp(0.0, 4.0) as f32,
            "opacity" => clip.properties.opacity = number()?.clamp(0.0, 1.0) as f32,
            "zoom" => {
                let z = number()?.clamp(0.01, 100.0) as f32;
                clip.properties.scale_x = z;
                clip.properties.scale_y = z;
            }
            "fade_in" => clip.properties.fade_in_us = us(number()?).clamp(0, half),
            "fade_out" => clip.properties.fade_out_us = us(number()?).clamp(0, half),
            "enabled" => clip.properties.enabled = matches!(value, Value::Boolean(true)),
            other => return Err(script_error(format!("'{other}' is not a property a plugin can set"))),
        }
        Ok(())
    })?)?;
    tempo.set("timeline", timeline)?;

    // ---- Markers --------------------------------------------------------------------
    let markers = lua.create_table()?;
    let s = session.clone();
    markers.set("list", lua.create_function(move |lua, ()| {
        let s = s.borrow();
        need_read(&s)?;
        let list = lua.create_table()?;
        for m in &s.timeline.markers {
            let t = lua.create_table()?;
            t.set("id", m.id.to_string())?;
            t.set("time", secs(m.position_us))?;
            t.set("name", m.name.as_str())?;
            list.push(t)?;
        }
        Ok(list)
    })?)?;
    let s = session.clone();
    markers.set("add", lua.create_function(move |_, (time, name): (f64, Option<String>)| {
        let mut s = s.borrow_mut();
        need_write(&s)?;
        let marker = Marker::new(us(time).max(0), name.unwrap_or_default(), MarkerColor::Blue);
        let id = marker.id.to_string();
        run(&mut s, AddMarkerCommand::new(marker))?;
        Ok(id)
    })?)?;
    let s = session.clone();
    markers.set("remove", lua.create_function(move |_, id: String| {
        let mut s = s.borrow_mut();
        need_write(&s)?;
        run(&mut s, DeleteMarkerCommand::new(parse_id(&id)?))
    })?)?;
    tempo.set("markers", markers)?;

    // ---- Media ------------------------------------------------------------------------
    let media = lua.create_table()?;
    let s = session.clone();
    media.set("loudness", lua.create_function(move |_, (id, window): (String, f64)| {
        let s = s.borrow();
        need_read(&s)?;
        let (_, clip) = s.timeline.find_clip(parse_id(&id)?).ok_or_else(|| script_error("no such clip"))?;
        let Some(peaks) = loudness.get(&clip.source_id) else { return Ok(Vec::new()) };
        // One value per `window` seconds across the clip, in decibels (0 is full level).
        let window = window.clamp(1.0 / PEAKS_PER_SECOND, 10.0);
        let steps = (secs(clip.duration_us()) / window).ceil() as usize;
        let levels = (0..steps)
            .map(|i| {
                let t0 = secs(clip.source_in) + i as f64 * window;
                let a = (t0 * PEAKS_PER_SECOND) as usize;
                let b = (((t0 + window) * PEAKS_PER_SECOND) as usize).max(a + 1).min(peaks.len());
                let peak = peaks.get(a..b).and_then(|p| p.iter().max()).copied().unwrap_or(0);
                20.0 * (peak.max(1) as f64 / 255.0).log10()
            })
            .collect::<Vec<f64>>();
        Ok(levels)
    })?)?;
    tempo.set("media", media)?;

    lua.globals().set("tempo", tempo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::{CommandLog, ReplaceTimelineCommand};

    fn plugin(access: &str, script: &str) -> Plugin {
        let manifest = format!(
            "[plugin]\nid = \"test.plugin\"\nname = \"Test\"\n\n[permissions]\ntimeline = \"{access}\"\n\n[[command]]\nid = \"go\"\nname = \"Go\"\nfunction = \"go\"\n"
        );
        Plugin::from_sources(&manifest, script, false).expect("manifest")
    }

    fn input(timeline: Timeline) -> RunInput {
        RunInput { timeline, playhead_us: 2_000_000, selection: Vec::new(), loudness: HashMap::new() }
    }

    fn timeline_with_audio_clip() -> (Timeline, Uuid, Uuid) {
        let mut timeline = Timeline::new_default();
        let track = timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio).map(|t| t.id).expect("audio track");
        let source = Uuid::new_v4();
        let clip = Clip::new(track, source, ClipType::Audio, "voice", 0, 10_000_000, 0, 10_000_000);
        let id = clip.id;
        timeline.find_track_mut(track).expect("track").clips.push(clip);
        (timeline, id, source)
    }

    #[test]
    fn manifest_is_read_and_built_in_plugins_load() {
        let p = plugin("write", "function go() end");
        assert_eq!((p.id.as_str(), p.timeline, p.commands.len()), ("test.plugin", TimelineAccess::Write, 1));
        assert!(Plugin::from_sources("not toml at all [", "", false).is_err());
        assert_eq!(built_in().len(), 1);
    }

    #[test]
    fn an_endless_loop_is_stopped() {
        let started = Instant::now();
        let result = run_command(&plugin("write", "function go() while true do end end"), "go", input(Timeline::new_default()));
        assert!(matches!(result, Err(PluginError::Script(m)) if m.contains("too long")));
        assert!(started.elapsed() < TIME_LIMIT + Duration::from_secs(3));
    }

    #[test]
    fn the_sandbox_has_no_file_or_process_access() {
        let script = "function go() assert(io == nil and os == nil and require == nil and load == nil and dofile == nil) end";
        assert!(run_command(&plugin("write", script), "go", input(Timeline::new_default())).is_ok());
    }

    #[test]
    fn permissions_are_enforced() {
        let script = "function go() tempo.markers.add(1, 'x') end";
        let denied = run_command(&plugin("read", script), "go", input(Timeline::new_default()));
        assert!(matches!(denied, Err(PluginError::Script(m)) if m.contains("permission")));
        let none = run_command(&plugin("none", "function go() tempo.timeline.tracks() end"), "go", input(Timeline::new_default()));
        assert!(none.is_err());
    }

    #[test]
    fn many_changes_become_one_undo_step() {
        let script = "function go() for i = 1, 50 do tempo.markers.add(i, 'm' .. i) end tempo.notify('done') end";
        let mut timeline = Timeline::new_default();
        let before = timeline.clone();
        let out = run_command(&plugin("write", script), "go", input(timeline.clone())).expect("run");
        assert!(out.changed);
        assert_eq!(out.messages, vec!["done".to_string()]);
        assert_eq!(out.timeline.markers.len(), 50);

        let mut log = CommandLog::new();
        log.execute(Box::new(ReplaceTimelineCommand::new("Script", out.timeline)), &mut timeline).expect("apply");
        assert_eq!(timeline.markers.len(), 50);
        log.undo(&mut timeline).expect("undo");
        assert_eq!(timeline, before);
    }

    #[test]
    fn a_failing_script_changes_nothing() {
        let script = "function go() tempo.markers.add(1, 'kept?') error('boom') end";
        let result = run_command(&plugin("write", script), "go", input(Timeline::new_default()));
        // The caller only ever applies the timeline of a successful run.
        assert!(matches!(result, Err(PluginError::Script(m)) if m.contains("boom")));
    }

    #[test]
    fn remove_silence_cuts_the_quiet_middle() {
        let (timeline, clip, source) = timeline_with_audio_clip();
        // Ten seconds: loud, two silent seconds from 4 s to 6 s, loud again.
        let peaks: Vec<u8> = (0..500).map(|i| if (200..300).contains(&i) { 0 } else { 180 }).collect();
        let run = RunInput { timeline, playhead_us: 0, selection: vec![clip], loudness: HashMap::from([(source, Arc::new(peaks))]) };
        let out = run_command(&built_in()[0], "remove_silence", run).expect("run");
        assert!(out.changed, "{:?}", out.messages);
        let track = out.timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio).expect("audio track");
        let spans: Vec<(i64, i64)> = track.clips.iter().map(|c| (c.timeline_in, c.timeline_out)).collect();
        // Two clips remain, touching at 4 s, eight seconds in total.
        assert_eq!(spans, vec![(0, 4_000_000), (4_000_000, 8_000_000)]);
        assert!(out.messages[0].contains("Removed 1"));
    }
}
