//! Application state shared by every widget on the GTK thread.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use tempo_timeline::{Clip, Command, CommandLog, Project, Timeline, TrackKind};
use uuid::Uuid;

use crate::player::{Player, Snapshot};

/// What changed, so listeners redraw only what they must.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A project was opened or closed.
    Project,
    /// Clips, tracks or markers changed.
    Timeline,
    /// The Media Pool changed.
    Media,
    /// A different clip (or nothing) is selected.
    Selection,
    /// Tool, snapping, destination tracks, In/Out marks.
    Options,
    /// Saved / unsaved state flipped.
    Dirty,
    /// Waveform data for a source became available.
    Waveform,
    /// The set of enabled plugins changed.
    Plugins,
    /// Something the user should be told.
    Message(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Trim,
    Blade,
}

pub struct AppState {
    pub project: RefCell<Option<Project>>,
    pub path: RefCell<Option<PathBuf>>,
    log: RefCell<CommandLog>,
    /// The clip the Inspector shows: the one clicked last.
    pub selection: Cell<Option<Uuid>>,
    /// Other clips selected with it (Ctrl+click, linked partners, Select All).
    pub extra_selection: RefCell<Vec<Uuid>>,
    pub tool: Cell<Tool>,
    pub snapping: Cell<bool>,
    /// When on, selecting a clip also selects the clips linked to it.
    pub linked_selection: Cell<bool>,
    pub dirty: Cell<bool>,
    pub mark_in: Cell<Option<i64>>,
    pub mark_out: Cell<Option<i64>>,
    /// `kind_index` of the video / audio track that receives the next edit.
    pub dest_video: Cell<u32>,
    pub dest_audio: Cell<u32>,
    /// Media Pool item shown in the viewer's source mode, if any.
    pub source_clip: Cell<Option<Uuid>>,
    /// In / Out set on the source clip while it is open in the viewer.
    pub src_in: Cell<Option<i64>>,
    pub src_out: Cell<Option<i64>>,
    /// Item selected in the Media Pool.
    pub media_selection: Cell<Option<Uuid>>,
    /// The clip last copied or cut.
    pub clipboard: RefCell<Option<Clip>>,
    pub settings: RefCell<crate::settings::Settings>,
    /// True while the source clip has its own viewer beside the timeline viewer.
    pub dual_viewer: Cell<bool>,
    /// Filters offered by the enabled plugins, kept current by `Plugins`.
    pub filters: RefCell<Vec<tempo_timeline::ClipEffect>>,
    /// Upload targets offered by the enabled plugins, kept current by `Plugins`.
    pub uploaders: RefCell<Vec<crate::plugins::Uploader>>,
    /// Opens a Media Pool clip in whichever viewer shows source clips.
    pub open_source: RefCell<Option<Rc<dyn Fn(Option<Uuid>)>>>,
    /// Loudness of each source, 50 values a second, for drawing waveforms.
    pub waveforms: RefCell<std::collections::HashMap<Uuid, Rc<Vec<u8>>>>,
    pub player: Player,
    listeners: RefCell<Vec<Rc<dyn Fn(&Change)>>>,
}

impl AppState {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            project: RefCell::new(None),
            path: RefCell::new(None),
            log: RefCell::new(CommandLog::new()),
            selection: Cell::new(None),
            extra_selection: RefCell::new(Vec::new()),
            tool: Cell::new(Tool::Select),
            snapping: Cell::new(true),
            linked_selection: Cell::new(true),
            dirty: Cell::new(false),
            mark_in: Cell::new(None),
            mark_out: Cell::new(None),
            dest_video: Cell::new(1),
            dest_audio: Cell::new(1),
            source_clip: Cell::new(None),
            src_in: Cell::new(None),
            src_out: Cell::new(None),
            media_selection: Cell::new(None),
            clipboard: RefCell::new(None),
            settings: RefCell::new(Default::default()),
            dual_viewer: Cell::new(false),
            filters: RefCell::new(Vec::new()),
            uploaders: RefCell::new(Vec::new()),
            open_source: RefCell::new(None),
            waveforms: RefCell::new(Default::default()),
            player: Player::new(),
            listeners: RefCell::new(Vec::new()),
        })
    }

    pub fn connect(&self, f: impl Fn(&Change) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn emit(&self, change: Change) {
        // Clone the list first: a listener may register another listener.
        let listeners: Vec<_> = self.listeners.borrow().clone();
        for l in listeners {
            l(&change);
        }
    }

    pub fn message(&self, text: impl Into<String>) {
        let text = text.into();
        tracing::info!("message: {text}");
        self.emit(Change::Message(text));
    }

    /// Run `f` with the timeline, if a project is open.
    pub fn with_timeline<R>(&self, f: impl FnOnce(&Timeline) -> R) -> Option<R> {
        self.project.borrow().as_ref().map(|p| f(&p.timeline))
    }

    pub fn with_project<R>(&self, f: impl FnOnce(&Project) -> R) -> Option<R> {
        self.project.borrow().as_ref().map(f)
    }

    /// True when In/Out and the transport act on a source clip shown in the single viewer.
    pub fn source_mode(&self) -> bool {
        self.source_clip.get().is_some() && !self.dual_viewer.get()
    }

    pub fn show_source(&self, source: Option<Uuid>) {
        let open = self.open_source.borrow().clone();
        if let Some(open) = open {
            open(source);
        }
    }

    pub fn frame_us(&self) -> i64 {
        self.with_project(|p| p.fps.frame_duration_us()).unwrap_or(33_333).max(1)
    }

    pub fn fps(&self) -> f64 {
        self.with_project(|p| p.fps.to_f64()).unwrap_or(30.0)
    }

    pub fn set_project(&self, project: Option<Project>, path: Option<PathBuf>) {
        self.player.pause();
        self.player.set_source(None);
        *self.project.borrow_mut() = project;
        *self.path.borrow_mut() = path;
        self.log.borrow_mut().clear();
        self.selection.set(None);
        self.extra_selection.borrow_mut().clear();
        self.source_clip.set(None);
        self.waveforms.borrow_mut().clear();
        self.media_selection.set(None);
        self.src_in.set(None);
        self.src_out.set(None);
        self.mark_in.set(None);
        self.mark_out.set(None);
        self.dest_video.set(1);
        self.dest_audio.set(1);
        self.dirty.set(false);
        self.sync_player();
        self.player.seek(0, true);
        self.emit(Change::Project);
    }

    /// Hand the playback worker a fresh copy of the timeline.
    pub fn sync_player(&self) {
        if let Some(p) = self.project.borrow().as_ref() {
            self.player.set_timeline(Snapshot::from_project(p));
        }
    }

    fn after_edit(&self) {
        self.sync_player();
        // Forget selected clips that no longer exist.
        let gone = |id: &Uuid| self.with_timeline(|t| t.find_clip(*id).is_none()).unwrap_or(true);
        let before = self.selected_ids();
        self.extra_selection.borrow_mut().retain(|id| !gone(id));
        if self.selection.get().is_some_and(|id| gone(&id)) {
            let next = self.extra_selection.borrow_mut().pop();
            self.selection.set(next);
        }
        if before != self.selected_ids() {
            self.emit(Change::Selection);
        }
        self.set_dirty(true);
        self.emit(Change::Timeline);
    }

    pub fn set_dirty(&self, dirty: bool) {
        if self.dirty.replace(dirty) != dirty {
            self.emit(Change::Dirty);
        }
    }

    /// Every timeline change goes through here so it can be undone.
    pub fn execute(&self, command: Box<dyn Command>) -> bool {
        let result = {
            let mut project = self.project.borrow_mut();
            let Some(project) = project.as_mut() else { return false };
            self.log.borrow_mut().execute(command, &mut project.timeline)
        };
        match result {
            Ok(()) => {
                self.after_edit();
                true
            }
            Err(e) => {
                self.message(friendly_error(&e));
                false
            }
        }
    }

    /// Apply a command without recording it, to preview a change while the user
    /// is still dragging. Follow it with `commit_preview`.
    pub fn apply_unlogged(&self, mut command: Box<dyn Command>) {
        let applied = self.project.borrow_mut().as_mut().is_some_and(|p| command.execute(&mut p.timeline).is_ok());
        if applied {
            self.sync_player();
            self.emit(Change::Timeline);
        }
    }

    /// End a previewed drag: put the clip back as it was, then record the final
    /// state as one undo step.
    pub fn commit_preview(&self, name: &str, original: Clip) {
        let id = original.id;
        let Some(now) = self.with_timeline(|t| t.find_clip(id).map(|(_, c)| c.clone())).flatten() else { return };
        if now == original {
            return;
        }
        if let Some(clip) = self.project.borrow_mut().as_mut().and_then(|p| p.timeline.find_clip_mut(id)) {
            *clip = original;
        }
        self.execute(Box::new(tempo_timeline::EditClipCommand::new(name, now)));
    }

    pub fn undo(&self) {
        self.history_step(true);
    }

    pub fn redo(&self) {
        self.history_step(false);
    }

    fn history_step(&self, undo: bool) {
        let result = {
            let mut project = self.project.borrow_mut();
            let Some(project) = project.as_mut() else { return };
            let mut log = self.log.borrow_mut();
            if undo {
                log.undo(&mut project.timeline)
            } else {
                log.redo(&mut project.timeline)
            }
        };
        match result {
            Ok(()) => self.after_edit(),
            Err(e) => self.message(friendly_error(&e)),
        }
    }

    /// Every selected clip, the primary one first.
    pub fn selected_ids(&self) -> Vec<Uuid> {
        self.selection.get().into_iter().chain(self.extra_selection.borrow().iter().copied()).collect()
    }

    pub fn is_selected(&self, id: Uuid) -> bool {
        self.selection.get() == Some(id) || self.extra_selection.borrow().contains(&id)
    }

    /// Clips linked to `id` (its sound or its picture), if linked selection is on.
    fn linked_to(&self, id: Uuid) -> Vec<Uuid> {
        if !self.linked_selection.get() {
            return Vec::new();
        }
        self.with_timeline(|t| {
            let Some(link) = t.find_clip(id).and_then(|(_, c)| c.properties.link) else { return Vec::new() };
            t.tracks.iter().flat_map(|tr| tr.clips.iter()).filter(|c| c.id != id && c.properties.link == Some(link)).map(|c| c.id).collect()
        })
        .unwrap_or_default()
    }

    /// Select one clip (and what is linked to it), or nothing.
    pub fn select(&self, clip: Option<Uuid>) {
        let before = self.selected_ids();
        self.selection.set(clip);
        *self.extra_selection.borrow_mut() = clip.map(|id| self.linked_to(id)).unwrap_or_default();
        if before != self.selected_ids() {
            self.emit(Change::Selection);
        }
    }

    /// Add a clip to the selection, or take it out (Ctrl+click).
    pub fn toggle_select(&self, id: Uuid) {
        let mut group = vec![id];
        group.extend(self.linked_to(id));
        if self.is_selected(id) {
            let mut rest: Vec<Uuid> = self.selected_ids().into_iter().filter(|s| !group.contains(s)).collect();
            self.selection.set(if rest.is_empty() { None } else { Some(rest.remove(0)) });
            *self.extra_selection.borrow_mut() = rest;
        } else {
            let mut all = self.selected_ids();
            all.retain(|s| !group.contains(s));
            self.selection.set(Some(id));
            all.extend(group.into_iter().skip(1));
            *self.extra_selection.borrow_mut() = all;
        }
        self.emit(Change::Selection);
    }

    pub fn select_all(&self) {
        let mut all: Vec<Uuid> = self
            .with_timeline(|t| t.tracks.iter().filter(|tr| !tr.locked).flat_map(|tr| tr.clips.iter().map(|c| c.id)).collect())
            .unwrap_or_default();
        self.selection.set(if all.is_empty() { None } else { Some(all.remove(0)) });
        *self.extra_selection.borrow_mut() = all;
        self.emit(Change::Selection);
    }

    pub fn selected_clip(&self) -> Option<Clip> {
        let id = self.selection.get()?;
        self.with_timeline(|t| t.find_clip(id).map(|(_, c)| c.clone())).flatten()
    }

    /// Id of the destination track of the given kind.
    pub fn dest_track(&self, kind: TrackKind) -> Option<Uuid> {
        let index = match kind {
            TrackKind::Video => self.dest_video.get(),
            TrackKind::Audio => self.dest_audio.get(),
        };
        self.with_timeline(|t| {
            t.tracks.iter().find(|tr| tr.kind == kind && tr.kind_index == index).map(|tr| tr.id)
        })
        .flatten()
    }
}

fn friendly_error(e: &tempo_timeline::TimelineError) -> String {
    use tempo_timeline::TimelineError as E;
    match e {
        E::TrackLocked(_) => "That track is locked.".into(),
        E::ClipCollision(..) => "There is already a clip there.".into(),
        E::InvalidTimeRange(..) => "The clip cannot be trimmed that far.".into(),
        E::InvalidSplitPosition(..) => "The clip cannot be cut there.".into(),
        E::NothingToUndo => "Nothing to undo.".into(),
        E::NothingToRedo => "Nothing to redo.".into(),
        other => other.to_string(),
    }
}

/// `HH:MM:SS:FF` for a position, using the project frame rate.
pub fn timecode(us: i64, fps: f64) -> String {
    let fps_i = fps.round().max(1.0) as i64;
    let total_frames = (us.max(0) as f64 * fps / 1_000_000.0).floor() as i64;
    let frames = total_frames % fps_i;
    let secs = total_frames / fps_i;
    format!("{:02}:{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60, frames)
}

#[cfg(test)]
mod tests {
    use super::timecode;

    #[test]
    fn timecode_uses_project_rate() {
        assert_eq!(timecode(0, 30.0), "00:00:00:00");
        assert_eq!(timecode(1_000_000, 24.0), "00:00:01:00");
        assert_eq!(timecode(1_500_000, 24.0), "00:00:01:12");
        assert_eq!(timecode(3_661_000_000, 30.0), "01:01:01:00");
    }
}
