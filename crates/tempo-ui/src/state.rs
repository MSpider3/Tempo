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
    pub selection: Cell<Option<Uuid>>,
    pub tool: Cell<Tool>,
    pub snapping: Cell<bool>,
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
            tool: Cell::new(Tool::Select),
            snapping: Cell::new(true),
            dirty: Cell::new(false),
            mark_in: Cell::new(None),
            mark_out: Cell::new(None),
            dest_video: Cell::new(1),
            dest_audio: Cell::new(1),
            source_clip: Cell::new(None),
            src_in: Cell::new(None),
            src_out: Cell::new(None),
            media_selection: Cell::new(None),
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
        self.emit(Change::Message(text.into()));
    }

    /// Run `f` with the timeline, if a project is open.
    pub fn with_timeline<R>(&self, f: impl FnOnce(&Timeline) -> R) -> Option<R> {
        self.project.borrow().as_ref().map(|p| f(&p.timeline))
    }

    pub fn with_project<R>(&self, f: impl FnOnce(&Project) -> R) -> Option<R> {
        self.project.borrow().as_ref().map(f)
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
        self.source_clip.set(None);
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
        if let Some(id) = self.selection.get() {
            if self.with_timeline(|t| t.find_clip(id).is_none()).unwrap_or(true) {
                self.selection.set(None);
                self.emit(Change::Selection);
            }
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

    pub fn select(&self, clip: Option<Uuid>) {
        if self.selection.replace(clip) != clip {
            self.emit(Change::Selection);
        }
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
