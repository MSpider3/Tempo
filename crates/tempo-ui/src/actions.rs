//! Editing actions. Each builds commands and sends them through `AppState::execute`,
//! so every one of them can be undone.

use std::rc::Rc;

use tempo_timeline::{
    AddMarkerCommand, Clip, ClipType, Command, CompositeCommand, DeleteClipCommand, DeleteMarkerCommand,
    InsertClipCommand, Marker, MarkerColor, MediaType, MoveClipCommand, OverwriteClipCommand, PropertyChange,
    ReplaceClipCommand, RippleDeleteCommand, SetClipPropertyCommand, SplitClipCommand, TrackKind, TrimClipCommand,
    TrimEdge,
};
use uuid::Uuid;

use crate::state::{AppState, Change};

/// Still images get this length when placed on the timeline.
const STILL_DURATION_US: i64 = 5_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Insert,
    Overwrite,
    Append,
    OnTop,
}

/// Build the clip(s) a source produces: picture on a video track, sound on an audio track.
fn clips_for_source(
    state: &AppState,
    source_id: Uuid,
    at_us: i64,
    video_track: Option<Uuid>,
    audio_track: Option<Uuid>,
) -> Vec<Clip> {
    let Some((media_type, duration, has_audio, name)) = state
        .with_project(|p| {
            p.sources.get(&source_id).map(|s| {
                let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                (s.media_type, s.duration_us, s.audio_channels.is_some(), name)
            })
        })
        .flatten()
    else {
        return Vec::new();
    };

    // Honour In/Out marks set on this clip in the viewer's source mode.
    let marked = state.source_clip.get() == Some(source_id);
    let full = if media_type == MediaType::Image { STILL_DURATION_US } else { duration.max(1) };
    let src_in = if marked { state.src_in.get().unwrap_or(0).clamp(0, full - 1) } else { 0 };
    let src_out = if marked { state.src_out.get().unwrap_or(full).clamp(src_in + 1, full) } else { full };
    let len = src_out - src_in;

    let mut clips = Vec::new();
    let mut push = |track: Option<Uuid>, kind: ClipType| {
        if let Some(track) = track {
            clips.push(Clip::new(track, source_id, kind, name.clone(), at_us, at_us + len, src_in, src_out));
        }
    };
    match media_type {
        MediaType::Video => {
            push(video_track, ClipType::Video);
            if has_audio {
                push(audio_track, ClipType::Audio);
            }
        }
        MediaType::Image => push(video_track, ClipType::Image),
        MediaType::Audio => push(audio_track, ClipType::Audio),
    }
    clips
}

/// Place a Media Pool source on the timeline. `track` overrides the destination
/// track of its kind (used when a clip is dropped on a particular row).
pub fn place_source(state: &Rc<AppState>, source_id: Uuid, how: Place, at_us: Option<i64>, track: Option<Uuid>) {
    let mut video = state.dest_track(TrackKind::Video);
    let mut audio = state.dest_track(TrackKind::Audio);
    if let Some(t) = track {
        match state.with_timeline(|tl| tl.find_track(t).map(|tr| tr.kind)).flatten() {
            Some(TrackKind::Video) => video = Some(t),
            Some(TrackKind::Audio) => audio = Some(t),
            None => {}
        }
    }

    let playhead = at_us.unwrap_or_else(|| state.player.position_us());
    let frame = state.frame_us();
    let mut at = playhead - playhead % frame;

    if how == Place::Append {
        at = state
            .with_timeline(|t| {
                t.tracks.iter().filter(|tr| Some(tr.id) == video || Some(tr.id) == audio).map(|tr| tr.duration_us()).max()
            })
            .flatten()
            .unwrap_or(0);
    }

    let mut clips = clips_for_source(state, source_id, at, video, audio);
    if clips.is_empty() {
        state.message("Nothing to place: pick a clip in the Media Pool first.");
        return;
    }

    if how == Place::OnTop {
        // Lowest video track that is free over the clip's range.
        let (start, end) = (clips[0].timeline_in, clips[0].timeline_out);
        let free = state
            .with_timeline(|t| {
                let mut tracks: Vec<_> = t.tracks.iter().filter(|tr| tr.kind == TrackKind::Video && !tr.locked).collect();
                tracks.sort_by_key(|tr| tr.kind_index);
                tracks.into_iter().find(|tr| !tr.has_collision(start, end, None)).map(|tr| tr.id)
            })
            .flatten();
        let Some(free) = free else {
            state.message("No free video track at the playhead.");
            return;
        };
        clips.retain(|c| c.clip_type != ClipType::Audio);
        for c in &mut clips {
            c.track_id = free;
        }
    }

    let first = clips[0].id;
    let end = clips[0].timeline_out;
    let commands: Vec<Box<dyn Command>> = clips
        .into_iter()
        .map(|c| -> Box<dyn Command> {
            match how {
                Place::Insert => Box::new(InsertClipCommand::new_ripple(c)),
                Place::Overwrite => Box::new(OverwriteClipCommand::new(c)),
                Place::Append | Place::OnTop => Box::new(InsertClipCommand::new(c)),
            }
        })
        .collect();
    let name = match how {
        Place::Insert => "Insert",
        Place::Overwrite => "Overwrite",
        Place::Append => "Append",
        Place::OnTop => "Place on Top",
    };
    if state.execute(Box::new(CompositeCommand::new(name, commands))) {
        state.select(Some(first));
        if how != Place::OnTop {
            state.player.seek(end, true);
        }
    }
}

/// The source the edit keys act on: the clip open in the viewer, else the Media Pool selection.
pub fn current_source(state: &AppState) -> Option<Uuid> {
    state.source_clip.get().or(state.media_selection.get())
}

pub fn place_current(state: &Rc<AppState>, how: Place) {
    match current_source(state) {
        Some(id) => place_source(state, id, how, None, None),
        None => state.message("Pick a clip in the Media Pool first."),
    }
}

fn selected(state: &AppState) -> Option<(Uuid, Clip)> {
    let id = state.selection.get()?;
    state.with_timeline(|t| t.find_clip(id).map(|(tr, c)| (tr.id, c.clone()))).flatten()
}

pub fn delete_selected(state: &Rc<AppState>, ripple: bool) {
    let Some((track, clip)) = selected(state) else { return };
    let cmd: Box<dyn Command> = if ripple {
        Box::new(RippleDeleteCommand::new(track, clip.id))
    } else {
        Box::new(DeleteClipCommand::new(track, clip.id))
    };
    state.execute(cmd);
}

/// Turn the selected clip off or on (Resolve's `D`).
pub fn toggle_enabled(state: &Rc<AppState>) {
    if let Some((_, clip)) = selected(state) {
        state.execute(Box::new(SetClipPropertyCommand::new(clip.id, PropertyChange::Enabled(!clip.properties.enabled))));
    }
}

/// Copy the selected clip. With `cut` it is also removed, leaving a gap.
pub fn copy_selected(state: &Rc<AppState>, cut: bool) {
    let Some((track, clip)) = selected(state) else { return };
    *state.clipboard.borrow_mut() = Some(clip.clone());
    if cut {
        state.execute(Box::new(DeleteClipCommand::new(track, clip.id)));
    }
}

/// Paste the copied clip at the playhead, on the destination track of its kind.
pub fn paste(state: &Rc<AppState>) {
    let Some(mut clip) = state.clipboard.borrow().clone() else { return };
    let kind = if clip.clip_type == ClipType::Audio { TrackKind::Audio } else { TrackKind::Video };
    let Some(track) = state.dest_track(kind) else { return };
    let pos = state.player.position_us();
    let at = pos - pos % state.frame_us();
    let len = clip.duration_us();
    clip.id = Uuid::new_v4();
    clip.track_id = track;
    clip.timeline_in = at;
    clip.timeline_out = at + len;
    let id = clip.id;
    if state.execute(Box::new(OverwriteClipCommand::new(clip))) {
        state.select(Some(id));
        state.player.seek(at + len, true);
    }
}

/// Replace the selected timeline clip with the current Media Pool clip, keeping its place and length.
pub fn replace_selected(state: &Rc<AppState>) {
    let (Some((track, clip)), Some(source)) = (selected(state), current_source(state)) else {
        state.message("Select a clip on the timeline and a clip in the Media Pool first.");
        return;
    };
    let Some((name, duration)) = state
        .with_project(|p| {
            p.sources.get(&source).map(|s| (s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), s.duration_us))
        })
        .flatten()
    else {
        return;
    };
    let src_in = if state.source_clip.get() == Some(source) { state.src_in.get().unwrap_or(0) } else { 0 };
    if duration - src_in < clip.duration_us() {
        state.message("The new clip is too short to replace this one.");
        return;
    }
    state.execute(Box::new(ReplaceClipCommand::new(track, clip.id, source, src_in, name)));
}

/// Cut every unlocked clip under the playhead (Resolve's Razor).
pub fn razor(state: &Rc<AppState>) {
    let pos = state.player.position_us();
    let cuts: Vec<Box<dyn Command>> = state
        .with_timeline(|t| {
            t.tracks
                .iter()
                .filter(|tr| !tr.locked)
                .filter_map(|tr| {
                    tr.clips
                        .iter()
                        .find(|c| pos > c.timeline_in && pos < c.timeline_out)
                        .map(|c| Box::new(SplitClipCommand::new(tr.id, c.id, pos)) as Box<dyn Command>)
                })
                .collect()
        })
        .unwrap_or_default();
    if !cuts.is_empty() {
        state.execute(Box::new(CompositeCommand::new("Razor", cuts)));
    }
}

pub fn split_at(state: &Rc<AppState>, clip_id: Uuid, at_us: i64) {
    let track = state.with_timeline(|t| t.find_clip(clip_id).map(|(tr, _)| tr.id)).flatten();
    if let Some(track) = track {
        state.execute(Box::new(SplitClipCommand::new(track, clip_id, at_us)));
    }
}

pub fn split_selected(state: &Rc<AppState>) {
    if let Some((_, clip)) = selected(state) {
        split_at(state, clip.id, state.player.position_us());
    }
}

/// Trim the start or end of the selected clip to the playhead.
pub fn trim_to_playhead(state: &Rc<AppState>, edge: TrimEdge) {
    let Some((_, clip)) = selected(state) else { return };
    let pos = state.player.position_us();
    let delta = match edge {
        TrimEdge::In => pos - clip.timeline_in,
        TrimEdge::Out => pos - clip.timeline_out,
    };
    if delta != 0 {
        state.execute(Box::new(TrimClipCommand::new(clip.id, edge, delta)));
    }
}

pub fn nudge(state: &Rc<AppState>, frames: i64) {
    let Some((track, clip)) = selected(state) else { return };
    let new_in = (clip.timeline_in + frames * state.frame_us()).max(0);
    if new_in != clip.timeline_in {
        state.execute(Box::new(MoveClipCommand::new(clip.id, track, track, clip.timeline_in, new_in)));
    }
}

/// Move the selected clip one track up (`+1`) or down (`-1`) within its kind.
pub fn move_track(state: &Rc<AppState>, step: i32) {
    let Some((track, clip)) = selected(state) else { return };
    let target = state
        .with_timeline(|t| {
            let cur = t.find_track(track)?;
            let want = cur.kind_index as i32 + step;
            t.tracks.iter().find(|tr| tr.kind == cur.kind && tr.kind_index as i32 == want).map(|tr| tr.id)
        })
        .flatten();
    if let Some(target) = target {
        state.execute(Box::new(MoveClipCommand::new(clip.id, track, target, clip.timeline_in, clip.timeline_in)));
    }
}

// ---- Markers ---------------------------------------------------------------

pub fn marker_at_playhead(state: &AppState) -> Option<Marker> {
    let pos = state.player.position_us();
    let frame = state.frame_us();
    state.with_timeline(|t| t.markers.iter().find(|m| (m.position_us - pos).abs() < frame).cloned()).flatten()
}

pub fn add_marker(state: &Rc<AppState>) -> Option<Marker> {
    if let Some(existing) = marker_at_playhead(state) {
        return Some(existing);
    }
    let pos = state.player.position_us();
    let marker = Marker::new(pos - pos % state.frame_us(), "", MarkerColor::Blue);
    state.execute(Box::new(AddMarkerCommand::new(marker.clone()))).then_some(marker)
}

pub fn delete_marker(state: &Rc<AppState>, id: Uuid) {
    state.execute(Box::new(DeleteMarkerCommand::new(id)));
}

/// Replace a marker with an edited copy as one undo step.
pub fn update_marker(state: &Rc<AppState>, marker: Marker) {
    let cmds: Vec<Box<dyn Command>> =
        vec![Box::new(DeleteMarkerCommand::new(marker.id)), Box::new(AddMarkerCommand::new(marker))];
    state.execute(Box::new(CompositeCommand::new("Modify Marker", cmds)));
}

pub fn goto_marker(state: &Rc<AppState>, forward: bool) {
    let pos = state.player.position_us();
    let target = state
        .with_timeline(|t| {
            if forward {
                t.markers.iter().map(|m| m.position_us).find(|p| *p > pos)
            } else {
                t.markers.iter().rev().map(|m| m.position_us).find(|p| *p < pos)
            }
        })
        .flatten();
    if let Some(t) = target {
        state.player.pause();
        state.player.seek(t, true);
    }
}

// ---- Navigation and marks ----------------------------------------------------

/// Jump to the next or previous cut on any track.
pub fn goto_edit(state: &Rc<AppState>, forward: bool) {
    let pos = state.player.position_us();
    let target = state
        .with_timeline(|t| {
            let points = t.tracks.iter().flat_map(|tr| tr.clips.iter()).flat_map(|c| [c.timeline_in, c.timeline_out]);
            if forward {
                points.filter(|p| *p > pos).min()
            } else {
                points.filter(|p| *p < pos).max()
            }
        })
        .flatten();
    if let Some(t) = target {
        state.player.pause();
        state.player.seek(t, true);
    }
}

pub fn step_frames(state: &Rc<AppState>, frames: i64) {
    let frame = state.frame_us();
    let pos = state.player.position_us();
    let target = ((pos / frame) + frames).max(0) * frame;
    state.player.pause();
    state.player.seek(target.min(state.player.duration_us()), true);
}

/// Set (or clear) In / Out. In source mode the marks belong to the source clip.
pub fn set_mark(state: &Rc<AppState>, is_in: bool, clear: bool) {
    let value = (!clear).then(|| state.player.position_us());
    let source = state.source_clip.get().is_some();
    match (source, is_in) {
        (true, true) => state.src_in.set(value),
        (true, false) => state.src_out.set(value),
        (false, true) => state.mark_in.set(value),
        (false, false) => state.mark_out.set(value),
    }
    state.emit(Change::Options);
}

/// Mark In/Out around the clip under the playhead on the destination video track.
pub fn mark_clip(state: &Rc<AppState>) {
    let pos = state.player.position_us();
    let Some(track) = state.dest_track(TrackKind::Video) else { return };
    let range = state
        .with_timeline(|t| t.find_track(track).and_then(|tr| tr.clip_at(pos)).map(|c| (c.timeline_in, c.timeline_out)))
        .flatten();
    if let Some((a, b)) = range {
        state.mark_in.set(Some(a));
        state.mark_out.set(Some(b));
        state.emit(Change::Options);
    }
}
