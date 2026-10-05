use std::fmt::Debug;
use uuid::Uuid;
use crate::error::{Result, TimelineError};
use crate::types::{Clip, ClipType, Marker, Timeline};

pub trait Command: Send + Sync + Debug {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()>;
    fn undo(&mut self, timeline: &mut Timeline) -> Result<()>;
    fn description(&self) -> &str;
}

#[derive(Debug)]
pub struct CommandLog {
    history: Vec<Box<dyn Command>>,
    cursor: usize,
    max_size: usize,
}

impl Default for CommandLog {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandLog {
    pub fn new() -> Self {
        Self::with_max_size(100)
    }

    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            history: Vec::new(),
            cursor: 0,
            max_size,
        }
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.history.len()
    }

    pub fn execute(&mut self, mut command: Box<dyn Command>, timeline: &mut Timeline) -> Result<()> {
        command.execute(timeline)?;
        self.history.truncate(self.cursor);
        self.history.push(command);
        self.cursor += 1;
        if self.history.len() > self.max_size {
            self.history.remove(0);
            self.cursor = self.cursor.saturating_sub(1);
        }
        Ok(())
    }

    pub fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        if !self.can_undo() {
            return Err(TimelineError::NothingToUndo);
        }
        self.cursor -= 1;
        self.history[self.cursor].undo(timeline)
    }

    pub fn redo(&mut self, timeline: &mut Timeline) -> Result<()> {
        if !self.can_redo() {
            return Err(TimelineError::NothingToRedo);
        }
        let res = self.history[self.cursor].execute(timeline);
        if res.is_ok() {
            self.cursor += 1;
        }
        res
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    pub fn clear(&mut self) {
        self.history.clear();
        self.cursor = 0;
    }
}

// ----------------------------------------------------------------------------
// 1. InsertClipCommand
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct InsertClipCommand {
    clip: Clip,
    ripple: bool,
    previous_clips: Option<Vec<Clip>>,
    description: String,
}

impl InsertClipCommand {
    pub fn new(clip: Clip) -> Self {
        let desc = format!("Insert clip '{}'", clip.name);
        Self {
            clip,
            ripple: false,
            previous_clips: None,
            description: desc,
        }
    }

    pub fn new_ripple(clip: Clip) -> Self {
        let desc = format!("Ripple insert clip '{}'", clip.name);
        Self {
            clip,
            ripple: true,
            previous_clips: None,
            description: desc,
        }
    }

    pub fn with_ripple(mut self, ripple: bool) -> Self {
        self.ripple = ripple;
        self
    }

    pub fn clip_id(&self) -> Uuid {
        self.clip.id
    }
}

impl Command for InsertClipCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.clip.track_id)
            .ok_or(TimelineError::TrackNotFound(self.clip.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        if !self.ripple {
            if track.has_collision(self.clip.timeline_in, self.clip.timeline_out, None) {
                return Err(TimelineError::ClipCollision(track.id, self.clip.timeline_in));
            }
            track.clips.push(self.clip.clone());
            track.sort_clips();
            Ok(())
        } else {
            self.previous_clips = Some(track.clips.clone());
            let insert_in = self.clip.timeline_in;
            let insert_dur = self.clip.duration_us();
            let mut new_clips = Vec::new();

            for old in &track.clips {
                if old.timeline_out <= insert_in {
                    new_clips.push(old.clone());
                } else if old.timeline_in >= insert_in {
                    let mut shifted = old.clone();
                    let dur = shifted.duration_us();
                    shifted.timeline_in += insert_dur;
                    shifted.timeline_out = shifted.timeline_in + dur;
                    new_clips.push(shifted);
                } else {
                    let mut left = old.clone();
                    let left_dur = insert_in - old.timeline_in;
                    left.timeline_out = insert_in;
                    left.source_out = left.source_in + left_dur;
                    new_clips.push(left);

                    let mut right = old.clone();
                    right.id = Uuid::new_v4();
                    right.timeline_in = insert_in + insert_dur;
                    right.timeline_out = old.timeline_out + insert_dur;
                    right.source_in = old.source_in + left_dur;
                    new_clips.push(right);
                }
            }
            new_clips.push(self.clip.clone());
            track.clips = new_clips;
            track.sort_clips();
            Ok(())
        }
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.clip.track_id)
            .ok_or(TimelineError::TrackNotFound(self.clip.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        if let Some(prev) = self.previous_clips.take() {
            track.clips = prev;
            Ok(())
        } else if let Some(pos) = track.clips.iter().position(|c| c.id == self.clip.id) {
            track.clips.remove(pos);
            Ok(())
        } else {
            Err(TimelineError::ClipNotFound(self.clip.id))
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// 2. DeleteClipCommand
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct DeleteClipCommand {
    clip_id: Uuid,
    track_id: Uuid,
    deleted_clip: Option<Clip>,
    description: String,
}

impl DeleteClipCommand {
    pub fn new(track_id: Uuid, clip_id: Uuid) -> Self {
        Self {
            clip_id,
            track_id,
            deleted_clip: None,
            description: "Delete clip".to_string(),
        }
    }
}

impl Command for DeleteClipCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.track_id)
            .ok_or(TimelineError::TrackNotFound(self.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        if let Some(pos) = track.clips.iter().position(|c| c.id == self.clip_id) {
            let clip = track.clips.remove(pos);
            self.description = format!("Delete clip '{}'", clip.name);
            self.deleted_clip = Some(clip);
            Ok(())
        } else {
            Err(TimelineError::ClipNotFound(self.clip_id))
        }
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.track_id)
            .ok_or(TimelineError::TrackNotFound(self.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        if let Some(ref clip) = self.deleted_clip {
            track.clips.push(clip.clone());
            track.sort_clips();
            Ok(())
        } else {
            Err(TimelineError::ClipNotFound(self.clip_id))
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// 3. MoveClipCommand
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct MoveClipCommand {
    clip_id: Uuid,
    old_track_id: Uuid,
    new_track_id: Uuid,
    old_timeline_in: i64,
    new_timeline_in: i64,
    description: String,
}

impl MoveClipCommand {
    pub fn new(
        clip_id: Uuid,
        old_track_id: Uuid,
        new_track_id: Uuid,
        old_timeline_in: i64,
        new_timeline_in: i64,
    ) -> Self {
        Self {
            clip_id,
            old_track_id,
            new_track_id,
            old_timeline_in,
            new_timeline_in,
            description: "Move clip".to_string(),
        }
    }
}

impl Command for MoveClipCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let duration_us = {
            let (track, clip) = timeline
                .find_clip(self.clip_id)
                .ok_or(TimelineError::ClipNotFound(self.clip_id))?;

            if track.locked {
                return Err(TimelineError::TrackLocked(track.id));
            }
            clip.duration_us()
        };

        let new_timeline_out = self.new_timeline_in + duration_us;

        // Check collision on destination track
        let dest_track = timeline
            .find_track_mut(self.new_track_id)
            .ok_or(TimelineError::TrackNotFound(self.new_track_id))?;

        if dest_track.locked {
            return Err(TimelineError::TrackLocked(dest_track.id));
        }

        if dest_track.has_collision(
            self.new_timeline_in,
            new_timeline_out,
            Some(self.clip_id),
        ) {
            return Err(TimelineError::ClipCollision(dest_track.id, self.new_timeline_in));
        }

        // If moving between tracks, remove from source and add to dest
        if self.old_track_id != self.new_track_id {
            let mut clip = {
                let old_track = timeline
                    .find_track_mut(self.old_track_id)
                    .ok_or(TimelineError::TrackNotFound(self.old_track_id))?;
                let idx = old_track
                    .clips
                    .iter()
                    .position(|c| c.id == self.clip_id)
                    .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
                old_track.clips.remove(idx)
            };

            clip.track_id = self.new_track_id;
            clip.timeline_in = self.new_timeline_in;
            clip.timeline_out = new_timeline_out;

            let dest_track = timeline
                .find_track_mut(self.new_track_id)
                .ok_or(TimelineError::TrackNotFound(self.new_track_id))?;
            dest_track.clips.push(clip);
            dest_track.sort_clips();
        } else {
            let track = timeline
                .find_track_mut(self.old_track_id)
                .ok_or(TimelineError::TrackNotFound(self.old_track_id))?;
            let clip = track
                .find_clip_mut(self.clip_id)
                .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
            clip.timeline_in = self.new_timeline_in;
            clip.timeline_out = new_timeline_out;
            track.sort_clips();
        }

        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let duration_us = {
            let (_, clip) = timeline
                .find_clip(self.clip_id)
                .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
            clip.duration_us()
        };
        let old_timeline_out = self.old_timeline_in + duration_us;

        if self.old_track_id != self.new_track_id {
            let mut clip = {
                let curr_track = timeline
                    .find_track_mut(self.new_track_id)
                    .ok_or(TimelineError::TrackNotFound(self.new_track_id))?;
                let idx = curr_track
                    .clips
                    .iter()
                    .position(|c| c.id == self.clip_id)
                    .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
                curr_track.clips.remove(idx)
            };

            clip.track_id = self.old_track_id;
            clip.timeline_in = self.old_timeline_in;
            clip.timeline_out = old_timeline_out;

            let old_track = timeline
                .find_track_mut(self.old_track_id)
                .ok_or(TimelineError::TrackNotFound(self.old_track_id))?;
            old_track.clips.push(clip);
            old_track.sort_clips();
        } else {
            let track = timeline
                .find_track_mut(self.old_track_id)
                .ok_or(TimelineError::TrackNotFound(self.old_track_id))?;
            let clip = track
                .find_clip_mut(self.clip_id)
                .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
            clip.timeline_in = self.old_timeline_in;
            clip.timeline_out = old_timeline_out;
            track.sort_clips();
        }

        Ok(())
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// TrimClipCommand
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimEdge {
    In,
    Out,
}

#[derive(Debug)]
pub struct TrimClipCommand {
    pub clip_id: Uuid,
    pub edge: TrimEdge,
    pub delta_us: i64,
    saved_timeline_in: i64,
    saved_timeline_out: i64,
    saved_source_in: i64,
    saved_source_out: i64,
}

impl TrimClipCommand {
    pub fn new(clip_id: Uuid, edge: TrimEdge, delta_us: i64) -> Self {
        Self {
            clip_id,
            edge,
            delta_us,
            saved_timeline_in: 0,
            saved_timeline_out: 0,
            saved_source_in: 0,
            saved_source_out: 0,
        }
    }
}

impl Command for TrimClipCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let (track_id, (tin, tout, sin, sout)) = {
            let (track, clip) = timeline
                .find_clip(self.clip_id)
                .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
            (track.id, (clip.timeline_in, clip.timeline_out, clip.source_in, clip.source_out))
        };
        self.saved_timeline_in = tin;
        self.saved_timeline_out = tout;
        self.saved_source_in = sin;
        self.saved_source_out = sout;

        let (new_tin, new_tout, new_sin, new_sout) = match self.edge {
            TrimEdge::In => {
                let n_tin = tin + self.delta_us;
                let n_sin = sin + self.delta_us;
                if n_tin >= tout || n_sin < 0 {
                    return Err(TimelineError::InvalidTimeRange(n_tin, tout));
                }
                (n_tin, tout, n_sin, sout)
            }
            TrimEdge::Out => {
                let n_tout = tout + self.delta_us;
                let n_sout = sout + self.delta_us;
                if n_tout <= tin || n_sout <= sin {
                    return Err(TimelineError::InvalidTimeRange(tin, n_tout));
                }
                (tin, n_tout, sin, n_sout)
            }
        };

        // Collision detection on track
        let track = timeline
            .find_track_mut(track_id)
            .ok_or(TimelineError::TrackNotFound(track_id))?;
        for other in &track.clips {
            if other.id == self.clip_id {
                continue;
            }
            if new_tin < other.timeline_out && new_tout > other.timeline_in {
                return Err(TimelineError::ClipCollision(track_id, other.timeline_in));
            }
        }

        let clip = track
            .find_clip_mut(self.clip_id)
            .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
        clip.timeline_in = new_tin;
        clip.timeline_out = new_tout;
        clip.source_in = new_sin;
        clip.source_out = new_sout;
        track.sort_clips();
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let (track, _) = timeline
            .find_clip(self.clip_id)
            .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
        let track_id = track.id;

        let track = timeline
            .find_track_mut(track_id)
            .ok_or(TimelineError::TrackNotFound(track_id))?;
        let clip = track
            .find_clip_mut(self.clip_id)
            .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
        clip.timeline_in = self.saved_timeline_in;
        clip.timeline_out = self.saved_timeline_out;
        clip.source_in = self.saved_source_in;
        clip.source_out = self.saved_source_out;
        track.sort_clips();
        Ok(())
    }

    fn description(&self) -> &str {
        "Trim Clip"
    }
}

// ----------------------------------------------------------------------------
// 4. SplitClipCommand
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct SplitClipCommand {
    clip_id: Uuid,
    track_id: Uuid,
    split_time_us: i64,
    second_clip_id: Uuid,
    original_timeline_out: i64,
    original_source_out: i64,
    description: String,
}

impl SplitClipCommand {
    pub fn new(track_id: Uuid, clip_id: Uuid, split_time_us: i64) -> Self {
        Self {
            clip_id,
            track_id,
            split_time_us,
            second_clip_id: Uuid::new_v4(),
            original_timeline_out: 0,
            original_source_out: 0,
            description: "Split clip".to_string(),
        }
    }

    pub fn second_clip_id(&self) -> Uuid {
        self.second_clip_id
    }
}

impl Command for SplitClipCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.track_id)
            .ok_or(TimelineError::TrackNotFound(self.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        let clip = track
            .find_clip_mut(self.clip_id)
            .ok_or(TimelineError::ClipNotFound(self.clip_id))?;

        if self.split_time_us <= clip.timeline_in || self.split_time_us >= clip.timeline_out {
            return Err(TimelineError::InvalidSplitPosition(
                self.split_time_us,
                clip.timeline_in,
                clip.timeline_out,
            ));
        }

        self.original_timeline_out = clip.timeline_out;
        self.original_source_out = clip.source_out;

        let delta = self.split_time_us - clip.timeline_in;
        let split_source = clip.source_in + delta;

        let second_clip = Clip {
            id: self.second_clip_id,
            track_id: self.track_id,
            source_id: clip.source_id,
            clip_type: clip.clip_type,
            name: clip.name.clone(),
            timeline_in: self.split_time_us,
            timeline_out: self.original_timeline_out,
            source_in: split_source,
            source_out: self.original_source_out,
            properties: clip.properties.clone(),
            title_data: clip.title_data.clone(),
        };

        clip.timeline_out = self.split_time_us;
        clip.source_out = split_source;

        track.clips.push(second_clip);
        track.sort_clips();

        self.description = format!("Split clip at {}us", self.split_time_us);
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.track_id)
            .ok_or(TimelineError::TrackNotFound(self.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        // Remove second clip
        if let Some(pos) = track.clips.iter().position(|c| c.id == self.second_clip_id) {
            track.clips.remove(pos);
        }

        // Restore first clip range
        let first = track
            .find_clip_mut(self.clip_id)
            .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
        first.timeline_out = self.original_timeline_out;
        first.source_out = self.original_source_out;

        track.sort_clips();
        Ok(())
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// 5. SetClipPropertyCommand
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub enum PropertyChange {
    Enabled(bool),
    Opacity(f32),
    Volume(f32),
    Muted(bool),
    Position { x: f32, y: f32 },
    Scale { x: f32, y: f32 },
    Rotation(f32),
    Pan(f32),
}

#[derive(Debug)]
pub struct SetClipPropertyCommand {
    clip_id: Uuid,
    change: PropertyChange,
    old_change: Option<PropertyChange>,
    description: String,
}

impl SetClipPropertyCommand {
    pub fn new(clip_id: Uuid, change: PropertyChange) -> Self {
        let desc = format!("Change property on clip {}", clip_id);
        Self {
            clip_id,
            change,
            old_change: None,
            description: desc,
        }
    }
}

impl Command for SetClipPropertyCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let clip = timeline
            .find_clip_mut(self.clip_id)
            .ok_or(TimelineError::ClipNotFound(self.clip_id))?;

        match &self.change {
            PropertyChange::Enabled(val) => {
                self.old_change = Some(PropertyChange::Enabled(clip.properties.enabled));
                clip.properties.enabled = *val;
            }
            PropertyChange::Opacity(val) => {
                self.old_change = Some(PropertyChange::Opacity(clip.properties.opacity));
                clip.properties.opacity = *val;
            }
            PropertyChange::Volume(val) => {
                self.old_change = Some(PropertyChange::Volume(clip.properties.volume));
                clip.properties.volume = *val;
            }
            PropertyChange::Muted(val) => {
                self.old_change = Some(PropertyChange::Muted(clip.properties.muted));
                clip.properties.muted = *val;
            }
            PropertyChange::Position { x, y } => {
                self.old_change = Some(PropertyChange::Position {
                    x: clip.properties.position_x,
                    y: clip.properties.position_y,
                });
                clip.properties.position_x = *x;
                clip.properties.position_y = *y;
            }
            PropertyChange::Scale { x, y } => {
                self.old_change = Some(PropertyChange::Scale {
                    x: clip.properties.scale_x,
                    y: clip.properties.scale_y,
                });
                clip.properties.scale_x = *x;
                clip.properties.scale_y = *y;
            }
            PropertyChange::Rotation(val) => {
                self.old_change = Some(PropertyChange::Rotation(clip.properties.rotation));
                clip.properties.rotation = *val;
            }
            PropertyChange::Pan(val) => {
                self.old_change = Some(PropertyChange::Pan(clip.properties.pan));
                clip.properties.pan = *val;
            }
        }
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let old = self
            .old_change
            .as_ref()
            .ok_or_else(|| TimelineError::CommandFailed("No prior property state".to_string()))?;

        let clip = timeline
            .find_clip_mut(self.clip_id)
            .ok_or(TimelineError::ClipNotFound(self.clip_id))?;

        match old {
            PropertyChange::Enabled(val) => clip.properties.enabled = *val,
            PropertyChange::Opacity(val) => clip.properties.opacity = *val,
            PropertyChange::Volume(val) => clip.properties.volume = *val,
            PropertyChange::Muted(val) => clip.properties.muted = *val,
            PropertyChange::Position { x, y } => {
                clip.properties.position_x = *x;
                clip.properties.position_y = *y;
            }
            PropertyChange::Scale { x, y } => {
                clip.properties.scale_x = *x;
                clip.properties.scale_y = *y;
            }
            PropertyChange::Rotation(val) => clip.properties.rotation = *val,
            PropertyChange::Pan(val) => clip.properties.pan = *val,
        }
        Ok(())
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// 6. AddMarkerCommand
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct AddMarkerCommand {
    marker: Marker,
    description: String,
}

impl AddMarkerCommand {
    pub fn new(marker: Marker) -> Self {
        let desc = format!("Add marker '{}' at {}us", marker.name, marker.position_us);
        Self {
            marker,
            description: desc,
        }
    }
}

impl Command for AddMarkerCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        timeline.add_marker(self.marker.clone());
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        timeline
            .remove_marker(self.marker.id)
            .ok_or(TimelineError::MarkerNotFound(self.marker.id))?;
        Ok(())
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// 7. DeleteMarkerCommand
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct DeleteMarkerCommand {
    marker_id: Uuid,
    deleted_marker: Option<Marker>,
    description: String,
}

impl DeleteMarkerCommand {
    pub fn new(marker_id: Uuid) -> Self {
        Self {
            marker_id,
            deleted_marker: None,
            description: "Delete marker".to_string(),
        }
    }
}

impl Command for DeleteMarkerCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let marker = timeline
            .remove_marker(self.marker_id)
            .ok_or(TimelineError::MarkerNotFound(self.marker_id))?;
        self.description = format!("Delete marker '{}'", marker.name);
        self.deleted_marker = Some(marker);
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        if let Some(ref marker) = self.deleted_marker {
            timeline.add_marker(marker.clone());
            Ok(())
        } else {
            Err(TimelineError::MarkerNotFound(self.marker_id))
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// 9. DeleteWithGapCommand (Alias for DeleteClipCommand)
// ----------------------------------------------------------------------------

pub type DeleteWithGapCommand = DeleteClipCommand;

// ----------------------------------------------------------------------------
// 10. RippleDeleteCommand
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RippleDeleteCommand {
    track_id: Uuid,
    clip_id: Uuid,
    deleted_clip: Option<Clip>,
    previous_clips: Option<Vec<Clip>>,
    description: String,
}

impl RippleDeleteCommand {
    pub fn new(track_id: Uuid, clip_id: Uuid) -> Self {
        Self {
            track_id,
            clip_id,
            deleted_clip: None,
            previous_clips: None,
            description: "Ripple delete clip".to_string(),
        }
    }

    pub fn clip_id(&self) -> Uuid {
        self.clip_id
    }
}

impl Command for RippleDeleteCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.track_id)
            .ok_or(TimelineError::TrackNotFound(self.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        self.previous_clips = Some(track.clips.clone());

        let pos = track
            .clips
            .iter()
            .position(|c| c.id == self.clip_id)
            .ok_or(TimelineError::ClipNotFound(self.clip_id))?;

        let deleted = track.clips.remove(pos);
        let shift_amount = deleted.duration_us();
        let deleted_out = deleted.timeline_out;
        self.description = format!("Ripple delete clip '{}'", deleted.name);
        self.deleted_clip = Some(deleted);

        // Ripple all clips to the right of deleted clip
        for clip in track.clips.iter_mut() {
            if clip.timeline_in >= deleted_out {
                let dur = clip.duration_us();
                clip.timeline_in = (clip.timeline_in - shift_amount).max(0);
                clip.timeline_out = clip.timeline_in + dur;
            }
        }
        track.sort_clips();
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.track_id)
            .ok_or(TimelineError::TrackNotFound(self.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        if let Some(prev) = self.previous_clips.take() {
            track.clips = prev;
            Ok(())
        } else {
            Err(TimelineError::ClipNotFound(self.clip_id))
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// 11. OverwriteClipCommand
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct OverwriteClipCommand {
    clip: Clip,
    previous_clips: Option<Vec<Clip>>,
    description: String,
}

impl OverwriteClipCommand {
    pub fn new(clip: Clip) -> Self {
        let desc = format!("Overwrite clip '{}'", clip.name);
        Self {
            clip,
            previous_clips: None,
            description: desc,
        }
    }

    pub fn clip_id(&self) -> Uuid {
        self.clip.id
    }
}

impl Command for OverwriteClipCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.clip.track_id)
            .ok_or(TimelineError::TrackNotFound(self.clip.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        self.previous_clips = Some(track.clips.clone());

        let new_in = self.clip.timeline_in;
        let new_out = self.clip.timeline_out;
        let mut new_track_clips = Vec::new();

        for old in &track.clips {
            if old.timeline_out <= new_in || old.timeline_in >= new_out {
                // Completely outside overwrite range
                new_track_clips.push(old.clone());
            } else if old.timeline_in >= new_in && old.timeline_out <= new_out {
                // Completely covered by new clip -> overwritten/removed
                continue;
            } else if old.timeline_in < new_in && old.timeline_out > new_out {
                // Encloses new clip -> split into left and right parts
                let mut left = old.clone();
                let left_dur = new_in - old.timeline_in;
                left.timeline_out = new_in;
                left.source_out = left.source_in + left_dur;
                new_track_clips.push(left);

                let mut right = old.clone();
                right.id = Uuid::new_v4();
                let cut_offset = new_out - old.timeline_in;
                right.timeline_in = new_out;
                right.source_in = old.source_in + cut_offset;
                new_track_clips.push(right);
            } else if old.timeline_in < new_in && old.timeline_out > new_in {
                // Overlaps start -> trim end
                let mut left = old.clone();
                let left_dur = new_in - old.timeline_in;
                left.timeline_out = new_in;
                left.source_out = left.source_in + left_dur;
                new_track_clips.push(left);
            } else if old.timeline_in < new_out && old.timeline_out > new_out {
                // Overlaps end -> trim start
                let mut right = old.clone();
                let cut_offset = new_out - old.timeline_in;
                right.timeline_in = new_out;
                right.source_in = old.source_in + cut_offset;
                new_track_clips.push(right);
            }
        }

        new_track_clips.push(self.clip.clone());
        track.clips = new_track_clips;
        track.sort_clips();
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.clip.track_id)
            .ok_or(TimelineError::TrackNotFound(self.clip.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        if let Some(prev) = self.previous_clips.take() {
            track.clips = prev;
            Ok(())
        } else {
            Err(TimelineError::ClipNotFound(self.clip.id))
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// 12. ReplaceClipCommand
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReplaceClipCommand {
    track_id: Uuid,
    clip_id: Uuid,
    new_source_id: Uuid,
    new_source_in: i64,
    new_name: String,
    old_clip: Option<Clip>,
    description: String,
}

impl ReplaceClipCommand {
    pub fn new(
        track_id: Uuid,
        clip_id: Uuid,
        new_source_id: Uuid,
        new_source_in: i64,
        new_name: impl Into<String>,
    ) -> Self {
        let name = new_name.into();
        let desc = format!("Replace clip with '{}'", name);
        Self {
            track_id,
            clip_id,
            new_source_id,
            new_source_in,
            new_name: name,
            old_clip: None,
            description: desc,
        }
    }
}

impl Command for ReplaceClipCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.track_id)
            .ok_or(TimelineError::TrackNotFound(self.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        let clip = track
            .clips
            .iter_mut()
            .find(|c| c.id == self.clip_id)
            .ok_or(TimelineError::ClipNotFound(self.clip_id))?;

        self.old_clip = Some(clip.clone());
        let duration = clip.duration_us();
        clip.source_id = self.new_source_id;
        clip.source_in = self.new_source_in;
        clip.source_out = self.new_source_in + duration;
        clip.name = self.new_name.clone();

        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.track_id)
            .ok_or(TimelineError::TrackNotFound(self.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        if let Some(ref old) = self.old_clip {
            let clip = track
                .clips
                .iter_mut()
                .find(|c| c.id == self.clip_id)
                .ok_or(TimelineError::ClipNotFound(self.clip_id))?;
            *clip = old.clone();
            Ok(())
        } else {
            Err(TimelineError::ClipNotFound(self.clip_id))
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// 13. AppendClipCommand
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AppendClipCommand {
    clip: Clip,
    description: String,
}

impl AppendClipCommand {
    pub fn new(
        track_id: Uuid,
        source_id: Uuid,
        clip_type: ClipType,
        name: impl Into<String>,
        source_in: i64,
        source_out: i64,
    ) -> Self {
        let name = name.into();
        let desc = format!("Append clip '{}'", name);
        let clip = Clip::new(
            track_id,
            source_id,
            clip_type,
            name,
            0,
            source_out - source_in,
            source_in,
            source_out,
        );
        Self {
            clip,
            description: desc,
        }
    }

    pub fn clip_id(&self) -> Uuid {
        self.clip.id
    }
}

impl Command for AppendClipCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.clip.track_id)
            .ok_or(TimelineError::TrackNotFound(self.clip.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        let track_end = track.clips.last().map(|c| c.timeline_out).unwrap_or(0);
        let duration = self.clip.source_out - self.clip.source_in;
        self.clip.timeline_in = track_end;
        self.clip.timeline_out = track_end + duration;

        track.clips.push(self.clip.clone());
        track.sort_clips();
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track = timeline
            .find_track_mut(self.clip.track_id)
            .ok_or(TimelineError::TrackNotFound(self.clip.track_id))?;

        if track.locked {
            return Err(TimelineError::TrackLocked(track.id));
        }

        if let Some(pos) = track.clips.iter().position(|c| c.id == self.clip.id) {
            track.clips.remove(pos);
            Ok(())
        } else {
            Err(TimelineError::ClipNotFound(self.clip.id))
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// CompositeCommand — several commands as one undo step
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct CompositeCommand {
    commands: Vec<Box<dyn Command>>,
    description: String,
}

impl CompositeCommand {
    pub fn new(description: impl Into<String>, commands: Vec<Box<dyn Command>>) -> Self {
        Self {
            commands,
            description: description.into(),
        }
    }
}

impl Command for CompositeCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        for i in 0..self.commands.len() {
            if let Err(e) = self.commands[i].execute(timeline) {
                // Roll back the part that already ran so a failure changes nothing.
                for done in self.commands[..i].iter_mut().rev() {
                    let _ = done.undo(timeline);
                }
                return Err(e);
            }
        }
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        for cmd in self.commands.iter_mut().rev() {
            cmd.undo(timeline)?;
        }
        Ok(())
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// SetTrackFlagCommand — lock / enable a track
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackFlag {
    Locked,
    Enabled,
}

#[derive(Debug)]
pub struct SetTrackFlagCommand {
    track_id: Uuid,
    flag: TrackFlag,
    value: bool,
    old_value: bool,
}

impl SetTrackFlagCommand {
    pub fn new(track_id: Uuid, flag: TrackFlag, value: bool) -> Self {
        Self {
            track_id,
            flag,
            value,
            old_value: !value,
        }
    }

    fn apply(&self, timeline: &mut Timeline, value: bool) -> Result<bool> {
        let track = timeline
            .find_track_mut(self.track_id)
            .ok_or(TimelineError::TrackNotFound(self.track_id))?;
        let slot = match self.flag {
            TrackFlag::Locked => &mut track.locked,
            TrackFlag::Enabled => &mut track.enabled,
        };
        Ok(std::mem::replace(slot, value))
    }
}

impl Command for SetTrackFlagCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        self.old_value = self.apply(timeline, self.value)?;
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        self.apply(timeline, self.old_value)?;
        Ok(())
    }

    fn description(&self) -> &str {
        match self.flag {
            TrackFlag::Locked => "Lock track",
            TrackFlag::Enabled => "Enable track",
        }
    }
}

// ----------------------------------------------------------------------------
// RippleTrimCommand — trim an edge and move everything after it by the same amount
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct RippleTrimCommand {
    clip_id: Uuid,
    edge: TrimEdge,
    delta_us: i64,
    track_id: Option<Uuid>,
    previous_clips: Option<Vec<Clip>>,
}

impl RippleTrimCommand {
    /// `delta_us` moves the chosen edge: positive is later, negative is earlier.
    pub fn new(clip_id: Uuid, edge: TrimEdge, delta_us: i64) -> Self {
        Self { clip_id, edge, delta_us, track_id: None, previous_clips: None }
    }
}

impl Command for RippleTrimCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track_id = timeline.find_clip(self.clip_id).map(|(t, _)| t.id).ok_or(TimelineError::ClipNotFound(self.clip_id))?;
        let track = timeline.find_track_mut(track_id).ok_or(TimelineError::TrackNotFound(track_id))?;
        if track.locked {
            return Err(TimelineError::TrackLocked(track_id));
        }
        let before = track.clips.clone();
        let clip = track.find_clip_mut(self.clip_id).ok_or(TimelineError::ClipNotFound(self.clip_id))?;
        let old_out = clip.timeline_out;

        // The clip keeps its start on the timeline; only its length changes.
        let shift = match self.edge {
            TrimEdge::In => {
                let new_source_in = clip.source_in + self.delta_us;
                if new_source_in < 0 || new_source_in >= clip.source_out {
                    return Err(TimelineError::InvalidTimeRange(new_source_in, clip.source_out));
                }
                clip.source_in = new_source_in;
                clip.timeline_out -= self.delta_us;
                -self.delta_us
            }
            TrimEdge::Out => {
                let new_out = clip.timeline_out + self.delta_us;
                if new_out <= clip.timeline_in {
                    return Err(TimelineError::InvalidTimeRange(clip.timeline_in, new_out));
                }
                clip.timeline_out = new_out;
                clip.source_out += self.delta_us;
                self.delta_us
            }
        };
        for other in track.clips.iter_mut().filter(|c| c.id != self.clip_id && c.timeline_in >= old_out) {
            other.timeline_in += shift;
            other.timeline_out += shift;
        }
        track.sort_clips();
        self.track_id = Some(track_id);
        self.previous_clips = Some(before);
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let track_id = self.track_id.ok_or(TimelineError::ClipNotFound(self.clip_id))?;
        let track = timeline.find_track_mut(track_id).ok_or(TimelineError::TrackNotFound(track_id))?;
        if let Some(previous) = self.previous_clips.take() {
            track.clips = previous;
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Ripple Trim"
    }
}

// ----------------------------------------------------------------------------
// EditClipCommand — replace a clip's settings (fades, title text, link …)
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct EditClipCommand {
    after: Clip,
    before: Option<Clip>,
    description: String,
}

impl EditClipCommand {
    /// `after` is the clip as it should become. Its id, track and timing must be
    /// those of the existing clip; only its other fields may differ.
    pub fn new(description: impl Into<String>, after: Clip) -> Self {
        Self { after, before: None, description: description.into() }
    }
}

impl Command for EditClipCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        let clip = timeline.find_clip_mut(self.after.id).ok_or(TimelineError::ClipNotFound(self.after.id))?;
        self.before = Some(std::mem::replace(clip, self.after.clone()));
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        let before = self.before.take().ok_or_else(|| TimelineError::CommandFailed("nothing to restore".into()))?;
        let clip = timeline.find_clip_mut(before.id).ok_or(TimelineError::ClipNotFound(before.id))?;
        *clip = before;
        Ok(())
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ----------------------------------------------------------------------------
// ReplaceTimelineCommand — swap in a whole edited timeline as one undo step
// ----------------------------------------------------------------------------

/// Used for work that makes many changes at once, such as a plugin script.
#[derive(Debug)]
pub struct ReplaceTimelineCommand {
    other: Timeline,
    description: String,
}

impl ReplaceTimelineCommand {
    pub fn new(description: impl Into<String>, after: Timeline) -> Self {
        Self { other: after, description: description.into() }
    }
}

impl Command for ReplaceTimelineCommand {
    fn execute(&mut self, timeline: &mut Timeline) -> Result<()> {
        std::mem::swap(timeline, &mut self.other);
        Ok(())
    }

    fn undo(&mut self, timeline: &mut Timeline) -> Result<()> {
        std::mem::swap(timeline, &mut self.other);
        Ok(())
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ClipType, MarkerColor};

    #[test]
    fn test_insert_and_undo_redo_clip() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let source_id = Uuid::new_v4();

        let clip = Clip::new(
            track_id,
            source_id,
            ClipType::Video,
            "clip_01.mp4",
            0,
            5_000_000,
            0,
            5_000_000,
        );
        let clip_id = clip.id;

        let mut log = CommandLog::new();
        assert!(!log.can_undo());
        assert!(!log.can_redo());

        // Execute Insert
        let cmd = Box::new(InsertClipCommand::new(clip));
        log.execute(cmd, &mut timeline).expect("insert should succeed");

        assert_eq!(timeline.tracks[0].clips.len(), 1);
        assert_eq!(timeline.tracks[0].clips[0].id, clip_id);
        assert_eq!(timeline.duration_us(), 5_000_000);
        assert!(log.can_undo());
        assert!(!log.can_redo());

        // Undo Insert
        log.undo(&mut timeline).expect("undo should succeed");
        assert_eq!(timeline.tracks[0].clips.len(), 0);
        assert_eq!(timeline.duration_us(), 0);
        assert!(!log.can_undo());
        assert!(log.can_redo());

        // Redo Insert
        log.redo(&mut timeline).expect("redo should succeed");
        assert_eq!(timeline.tracks[0].clips.len(), 1);
        assert_eq!(timeline.tracks[0].clips[0].id, clip_id);
        assert!(log.can_undo());
        assert!(!log.can_redo());
    }

    #[test]
    fn test_collision_detection() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let source_id = Uuid::new_v4();

        let clip1 = Clip::new(
            track_id,
            source_id,
            ClipType::Video,
            "c1",
            1_000_000,
            3_000_000,
            0,
            2_000_000,
        );
        let clip2 = Clip::new(
            track_id,
            source_id,
            ClipType::Video,
            "c2",
            2_000_000,
            4_000_000,
            0,
            2_000_000,
        );

        let mut log = CommandLog::new();
        log.execute(Box::new(InsertClipCommand::new(clip1)), &mut timeline).unwrap();

        // Inserting overlapping clip2 should fail
        let res = log.execute(Box::new(InsertClipCommand::new(clip2)), &mut timeline);
        assert!(matches!(res, Err(TimelineError::ClipCollision(..))));
    }

    #[test]
    fn test_delete_clip_command() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let source_id = Uuid::new_v4();

        let clip = Clip::new(
            track_id,
            source_id,
            ClipType::Video,
            "to_delete",
            0,
            2_000_000,
            0,
            2_000_000,
        );
        let clip_id = clip.id;

        let mut log = CommandLog::new();
        log.execute(Box::new(InsertClipCommand::new(clip)), &mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 1);

        // Delete
        log.execute(Box::new(DeleteClipCommand::new(track_id, clip_id)), &mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 0);

        // Undo Delete
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 1);
        assert_eq!(timeline.tracks[0].clips[0].id, clip_id);

        // Redo Delete
        log.redo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 0);
    }

    #[test]
    fn test_move_clip_command() {
        let mut timeline = Timeline::new_default();
        let track_v1 = timeline.tracks[0].id;
        let track_v2 = timeline.tracks[1].id;
        let source_id = Uuid::new_v4();

        let clip = Clip::new(
            track_v1,
            source_id,
            ClipType::Video,
            "movable",
            0,
            3_000_000,
            0,
            3_000_000,
        );
        let clip_id = clip.id;

        let mut log = CommandLog::new();
        log.execute(Box::new(InsertClipCommand::new(clip)), &mut timeline).unwrap();

        // Move to V2 at position 5_000_000
        let move_cmd = MoveClipCommand::new(clip_id, track_v1, track_v2, 0, 5_000_000);
        log.execute(Box::new(move_cmd), &mut timeline).unwrap();

        assert_eq!(timeline.tracks[0].clips.len(), 0);
        assert_eq!(timeline.tracks[1].clips.len(), 1);
        assert_eq!(timeline.tracks[1].clips[0].timeline_in, 5_000_000);
        assert_eq!(timeline.tracks[1].clips[0].timeline_out, 8_000_000);

        // Undo move
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 1);
        assert_eq!(timeline.tracks[1].clips.len(), 0);
        assert_eq!(timeline.tracks[0].clips[0].timeline_in, 0);

        // Redo move
        log.redo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 0);
        assert_eq!(timeline.tracks[1].clips.len(), 1);
        assert_eq!(timeline.tracks[1].clips[0].timeline_in, 5_000_000);
    }

    #[test]
    fn test_split_clip_command() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let source_id = Uuid::new_v4();

        let clip = Clip::new(
            track_id,
            source_id,
            ClipType::Video,
            "to_split",
            1_000_000,
            5_000_000,
            10_000_000,
            14_000_000,
        );
        let clip_id = clip.id;

        let mut log = CommandLog::new();
        log.execute(Box::new(InsertClipCommand::new(clip)), &mut timeline).unwrap();

        // Split at 3_000_000
        let split_cmd = SplitClipCommand::new(track_id, clip_id, 3_000_000);
        let second_id = split_cmd.second_clip_id();
        log.execute(Box::new(split_cmd), &mut timeline).unwrap();

        assert_eq!(timeline.tracks[0].clips.len(), 2);
        let c1 = &timeline.tracks[0].clips[0];
        let c2 = &timeline.tracks[0].clips[1];

        assert_eq!(c1.id, clip_id);
        assert_eq!(c1.timeline_in, 1_000_000);
        assert_eq!(c1.timeline_out, 3_000_000);
        assert_eq!(c1.source_in, 10_000_000);
        assert_eq!(c1.source_out, 12_000_000);

        assert_eq!(c2.id, second_id);
        assert_eq!(c2.timeline_in, 3_000_000);
        assert_eq!(c2.timeline_out, 5_000_000);
        assert_eq!(c2.source_in, 12_000_000);
        assert_eq!(c2.source_out, 14_000_000);

        // Undo split
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 1);
        let restored = &timeline.tracks[0].clips[0];
        assert_eq!(restored.timeline_in, 1_000_000);
        assert_eq!(restored.timeline_out, 5_000_000);

        // Redo split
        log.redo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 2);
    }

    #[test]
    fn test_set_clip_property_command() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let source_id = Uuid::new_v4();

        let clip = Clip::new(
            track_id,
            source_id,
            ClipType::Video,
            "prop_test",
            0,
            1_000_000,
            0,
            1_000_000,
        );
        let clip_id = clip.id;

        let mut log = CommandLog::new();
        log.execute(Box::new(InsertClipCommand::new(clip)), &mut timeline).unwrap();

        // Change opacity to 0.4
        let prop_cmd = SetClipPropertyCommand::new(clip_id, PropertyChange::Opacity(0.4));
        log.execute(Box::new(prop_cmd), &mut timeline).unwrap();

        let (_, clip_ref) = timeline.find_clip(clip_id).unwrap();
        assert!((clip_ref.properties.opacity - 0.4).abs() < 0.001);

        // Undo
        log.undo(&mut timeline).unwrap();
        let (_, clip_ref) = timeline.find_clip(clip_id).unwrap();
        assert!((clip_ref.properties.opacity - 1.0).abs() < 0.001);

        // Redo
        log.redo(&mut timeline).unwrap();
        let (_, clip_ref) = timeline.find_clip(clip_id).unwrap();
        assert!((clip_ref.properties.opacity - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_marker_commands() {
        let mut timeline = Timeline::new_default();
        let marker = Marker::new(2_500_000, "Scene 1", MarkerColor::Green);
        let marker_id = marker.id;

        let mut log = CommandLog::new();
        log.execute(Box::new(AddMarkerCommand::new(marker)), &mut timeline).unwrap();

        assert_eq!(timeline.markers.len(), 1);
        assert_eq!(timeline.markers[0].name, "Scene 1");

        // Delete marker
        log.execute(Box::new(DeleteMarkerCommand::new(marker_id)), &mut timeline).unwrap();
        assert_eq!(timeline.markers.len(), 0);

        // Undo delete
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline.markers.len(), 1);

        // Redo delete
        log.redo(&mut timeline).unwrap();
        assert_eq!(timeline.markers.len(), 0);
    }

    #[test]
    fn test_command_log_execute_and_undo() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let mut log = CommandLog::new();

        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "c1", 0, 1_000_000, 0, 1_000_000);
        log.execute(Box::new(InsertClipCommand::new(clip)), &mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 1);

        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 0);
    }

    #[test]
    fn test_command_log_redo_after_undo() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let mut log = CommandLog::new();

        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "c1", 0, 1_000_000, 0, 1_000_000);
        log.execute(Box::new(InsertClipCommand::new(clip)), &mut timeline).unwrap();
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 0);

        log.redo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 1);
    }

    #[test]
    fn test_command_log_redo_cleared_on_new_command() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let mut log = CommandLog::new();

        let c1 = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "c1", 0, 1_000_000, 0, 1_000_000);
        let c2 = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "c2", 2_000_000, 3_000_000, 0, 1_000_000);

        log.execute(Box::new(InsertClipCommand::new(c1)), &mut timeline).unwrap();
        log.undo(&mut timeline).unwrap();
        assert!(log.can_redo());

        // Executing c2 must clear redo history
        log.execute(Box::new(InsertClipCommand::new(c2)), &mut timeline).unwrap();
        assert!(!log.can_redo());
    }

    #[test]
    fn test_command_log_max_size_trimming() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let mut log = CommandLog::with_max_size(3);

        for i in 0..5 {
            let c = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, format!("c{}", i), (i * 2) * 1_000_000, (i * 2 + 1) * 1_000_000, 0, 1_000_000);
            log.execute(Box::new(InsertClipCommand::new(c)), &mut timeline).unwrap();
        }

        assert_eq!(log.history.len(), 3);
        assert_eq!(log.cursor, 3);
    }

    #[test]
    fn test_insert_clip_cmd_execute_and_undo() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "test", 0, 2_000_000, 0, 2_000_000);
        let clip_id = clip.id;

        let mut cmd = InsertClipCommand::new(clip);
        cmd.execute(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 1);

        cmd.undo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 0);
        assert!(timeline.find_clip(clip_id).is_none());
    }

    #[test]
    fn test_delete_clip_cmd_execute_and_undo() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "test", 0, 2_000_000, 0, 2_000_000);
        let clip_id = clip.id;
        timeline.tracks[0].clips.push(clip);

        let mut cmd = DeleteClipCommand::new(track_id, clip_id);
        cmd.execute(&mut timeline).unwrap();
        assert!(timeline.find_clip(clip_id).is_none());

        cmd.undo(&mut timeline).unwrap();
        assert!(timeline.find_clip(clip_id).is_some());
    }

    #[test]
    fn test_move_clip_cmd_execute_and_undo() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "test", 0, 2_000_000, 0, 2_000_000);
        let clip_id = clip.id;
        timeline.tracks[0].clips.push(clip);

        let mut cmd = MoveClipCommand::new(clip_id, track_id, track_id, 0, 3_000_000);
        cmd.execute(&mut timeline).unwrap();
        let (_, c) = timeline.find_clip(clip_id).unwrap();
        assert_eq!(c.timeline_in, 3_000_000);
        assert_eq!(c.timeline_out, 5_000_000);

        cmd.undo(&mut timeline).unwrap();
        let (_, c) = timeline.find_clip(clip_id).unwrap();
        assert_eq!(c.timeline_in, 0);
        assert_eq!(c.timeline_out, 2_000_000);
    }

    #[test]
    fn test_trim_clip_cmd_in_edge() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "test", 1_000_000, 5_000_000, 0, 4_000_000);
        let clip_id = clip.id;
        timeline.tracks[0].clips.push(clip);

        // Trim in edge by +500_000 (shortening clip from left)
        let mut cmd = TrimClipCommand::new(clip_id, TrimEdge::In, 500_000);
        cmd.execute(&mut timeline).unwrap();

        let (_, c) = timeline.find_clip(clip_id).unwrap();
        assert_eq!(c.timeline_in, 1_500_000);
        assert_eq!(c.timeline_out, 5_000_000);
        assert_eq!(c.source_in, 500_000);
        assert_eq!(c.source_out, 4_000_000);

        // Undo
        cmd.undo(&mut timeline).unwrap();
        let (_, c) = timeline.find_clip(clip_id).unwrap();
        assert_eq!(c.timeline_in, 1_000_000);
        assert_eq!(c.timeline_out, 5_000_000);
        assert_eq!(c.source_in, 0);
    }

    #[test]
    fn test_trim_clip_cmd_out_edge() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "test", 1_000_000, 5_000_000, 0, 4_000_000);
        let clip_id = clip.id;
        timeline.tracks[0].clips.push(clip);

        // Trim out edge by -1_000_000 (shortening clip from right)
        let mut cmd = TrimClipCommand::new(clip_id, TrimEdge::Out, -1_000_000);
        cmd.execute(&mut timeline).unwrap();

        let (_, c) = timeline.find_clip(clip_id).unwrap();
        assert_eq!(c.timeline_in, 1_000_000);
        assert_eq!(c.timeline_out, 4_000_000);
        assert_eq!(c.source_in, 0);
        assert_eq!(c.source_out, 3_000_000);

        // Undo
        cmd.undo(&mut timeline).unwrap();
        let (_, c) = timeline.find_clip(clip_id).unwrap();
        assert_eq!(c.timeline_out, 5_000_000);
        assert_eq!(c.source_out, 4_000_000);
    }

    #[test]
    fn test_split_clip_cmd_and_undo() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "test", 0, 4_000_000, 0, 4_000_000);
        let clip_id = clip.id;
        timeline.tracks[0].clips.push(clip);

        let mut cmd = SplitClipCommand::new(track_id, clip_id, 2_000_000);
        cmd.execute(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 2);

        cmd.undo(&mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 1);
        let (_, c) = timeline.find_clip(clip_id).unwrap();
        assert_eq!(c.timeline_in, 0);
        assert_eq!(c.timeline_out, 4_000_000);
    }

    #[test]
    fn test_set_clip_property_volume_and_undo() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Audio, "test", 0, 4_000_000, 0, 4_000_000);
        let clip_id = clip.id;
        timeline.tracks[0].clips.push(clip);

        let mut cmd = SetClipPropertyCommand::new(clip_id, PropertyChange::Volume(0.5));
        cmd.execute(&mut timeline).unwrap();

        let (_, c) = timeline.find_clip(clip_id).unwrap();
        assert!((c.properties.volume - 0.5).abs() < 0.001);

        cmd.undo(&mut timeline).unwrap();
        let (_, c) = timeline.find_clip(clip_id).unwrap();
        assert!((c.properties.volume - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_20_undo_steps_return_to_empty_timeline() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let mut log = CommandLog::with_max_size(50);

        for i in 0..20 {
            let clip = Clip::new(
                track_id,
                Uuid::new_v4(),
                ClipType::Video,
                format!("clip_{}", i),
                (i * 2) * 1_000_000,
                (i * 2 + 1) * 1_000_000,
                0,
                1_000_000,
            );
            log.execute(Box::new(InsertClipCommand::new(clip)), &mut timeline).unwrap();
        }

        assert_eq!(timeline.tracks[0].clips.len(), 20);

        // Perform 20 undos
        for _ in 0..20 {
            log.undo(&mut timeline).unwrap();
        }

        assert_eq!(timeline.tracks[0].clips.len(), 0);
        assert!(!log.can_undo());
    }

    #[test]
    fn test_editing_operations() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let mut log = CommandLog::new();

        // 1. Append (F12)
        let c1 = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "clip1", 0, 3_000_000, 0, 3_000_000);
        log.execute(Box::new(AppendClipCommand::new(track_id, c1.source_id, c1.clip_type, "clip1", 0, 3_000_000)), &mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 1);
        assert_eq!(timeline.duration_us(), 3_000_000);

        // 2. Append (F12) second clip
        log.execute(Box::new(AppendClipCommand::new(track_id, Uuid::new_v4(), ClipType::Video, "clip2", 0, 3_000_000)), &mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 2);
        assert_eq!(timeline.tracks[0].clips[1].timeline_in, 3_000_000);
        assert_eq!(timeline.tracks[0].clips[1].timeline_out, 6_000_000);
        assert_eq!(timeline.duration_us(), 6_000_000);

        // 3. Insert with ripple (F9) at 2s with duration 2s
        let c_insert = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "clip_inserted", 2_000_000, 4_000_000, 0, 2_000_000);
        log.execute(Box::new(InsertClipCommand::new_ripple(c_insert)), &mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 4); // clip1 was split into [0..2s] and [4s..5s], c_insert is [2s..4s], clip2 shifted to [5s..8s]
        assert_eq!(timeline.duration_us(), 8_000_000);

        // 4. Overwrite (F10) at 1s..3s
        let c_overwrite = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "clip_overwritten", 1_000_000, 3_000_000, 0, 2_000_000);
        log.execute(Box::new(OverwriteClipCommand::new(c_overwrite)), &mut timeline).unwrap();

        // 5. Replace (F11) on first clip
        let first_id = timeline.tracks[0].clips[0].id;
        let new_src = Uuid::new_v4();
        log.execute(Box::new(ReplaceClipCommand::new(track_id, first_id, new_src, 500_000, "replaced_clip")), &mut timeline).unwrap();
        let (_, replaced) = timeline.find_clip(first_id).unwrap();
        assert_eq!(replaced.source_id, new_src);
        assert_eq!(replaced.source_in, 500_000);

        // 6. Blade (Split)
        let last_id = timeline.tracks[0].clips.last().unwrap().id;
        let split_time = timeline.tracks[0].clips.last().unwrap().timeline_in + 1_000_000;
        let orig_count = timeline.tracks[0].clips.len();
        log.execute(Box::new(SplitClipCommand::new(track_id, last_id, split_time)), &mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), orig_count + 1);

        // 7. Trim (T)
        let trim_target_id = timeline.tracks[0].clips[1].id;
        let old_out = timeline.tracks[0].clips[1].timeline_out;
        log.execute(Box::new(TrimClipCommand::new(trim_target_id, TrimEdge::Out, -500_000)), &mut timeline).unwrap();
        let (_, trimmed) = timeline.find_clip(trim_target_id).unwrap();
        assert_eq!(trimmed.timeline_out, old_out - 500_000);

        // 8. Move
        let move_clip_id = timeline.tracks[0].clips[0].id;
        let old_in = timeline.tracks[0].clips[0].timeline_in;
        let v2_track_id = timeline.tracks[1].id;
        log.execute(Box::new(MoveClipCommand::new(move_clip_id, track_id, v2_track_id, old_in, 10_000_000)), &mut timeline).unwrap();
        assert!(timeline.tracks[1].clips.iter().any(|c| c.id == move_clip_id));

        // 9. DeleteWithGap (Delete key)
        let delete_clip_id = timeline.tracks[1].clips[0].id;
        log.execute(Box::new(DeleteWithGapCommand::new(v2_track_id, delete_clip_id)), &mut timeline).unwrap();
        assert_eq!(timeline.tracks[1].clips.len(), 0);

        // 10. RippleDelete (Backspace key)
        let target_id = timeline.tracks[0].clips[0].id;
        let pre_ripple_count = timeline.tracks[0].clips.len();
        log.execute(Box::new(RippleDeleteCommand::new(track_id, target_id)), &mut timeline).unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), pre_ripple_count - 1);

        // Verify full undo of all operations returns timeline to empty state
        while log.can_undo() {
            log.undo(&mut timeline).unwrap();
        }
        assert_eq!(timeline.tracks[0].clips.len(), 0);
        assert_eq!(timeline.tracks[1].clips.len(), 0);
    }

    #[test]
    fn test_undo_redo_stress() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let mut log = CommandLog::with_max_size(100);

        // Apply 50 diverse operations
        for i in 0..50 {
            let start = (i as i64) * 2_000_000;
            let end = start + 1_500_000;
            let clip = Clip::new(
                track_id,
                Uuid::new_v4(),
                ClipType::Video,
                format!("clip_{}", i),
                start,
                end,
                0,
                1_500_000,
            );
            log.execute(Box::new(InsertClipCommand::new(clip)), &mut timeline).unwrap();
        }

        assert_eq!(timeline.tracks[0].clips.len(), 50);
        assert_eq!(log.cursor(), 50);

        // Undo all 50 operations
        for _ in 0..50 {
            log.undo(&mut timeline).unwrap();
        }

        assert_eq!(timeline.tracks[0].clips.len(), 0);
        assert!(!log.can_undo());
        assert!(log.can_redo());

        // Redo all 50 operations
        for _ in 0..50 {
            log.redo(&mut timeline).unwrap();
        }

        assert_eq!(timeline.tracks[0].clips.len(), 50);
        assert!(log.can_undo());
        assert!(!log.can_redo());
    }

    #[test]
    fn test_composite_command_is_one_undo_step_and_rolls_back() {
        let mut timeline = Timeline::new_default();
        let before = timeline.clone();
        let track_id = timeline.tracks[0].id;
        let a = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "a", 0, 1_000_000, 0, 1_000_000);
        let b = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "b", 1_000_000, 2_000_000, 0, 1_000_000);
        let mut log = CommandLog::new();
        log.execute(
            Box::new(CompositeCommand::new(
                "two",
                vec![Box::new(InsertClipCommand::new(a.clone())), Box::new(InsertClipCommand::new(b))],
            )),
            &mut timeline,
        )
        .unwrap();
        assert_eq!(timeline.tracks[0].clips.len(), 2);
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline, before);

        // Second insert collides with the first: nothing must remain.
        let clash = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "c", 500_000, 900_000, 0, 400_000);
        let res = log.execute(
            Box::new(CompositeCommand::new(
                "clash",
                vec![Box::new(InsertClipCommand::new(a)), Box::new(InsertClipCommand::new(clash))],
            )),
            &mut timeline,
        );
        assert!(res.is_err());
        assert_eq!(timeline, before);
    }

    #[test]
    fn test_set_track_flag_undo() {
        let mut timeline = Timeline::new_default();
        let before = timeline.clone();
        let id = timeline.tracks[0].id;
        let mut log = CommandLog::new();
        log.execute(Box::new(SetTrackFlagCommand::new(id, TrackFlag::Locked, true)), &mut timeline).unwrap();
        assert!(timeline.tracks[0].locked);
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline, before);
    }

    #[test]
    fn test_ripple_trim_moves_later_clips_and_undoes() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let src = Uuid::new_v4();
        let a = Clip::new(track_id, src, ClipType::Video, "a", 0, 4_000_000, 1_000_000, 5_000_000);
        let b = Clip::new(track_id, src, ClipType::Video, "b", 4_000_000, 6_000_000, 0, 2_000_000);
        let (a_id, b_id) = (a.id, b.id);
        timeline.tracks[0].clips = vec![a, b];
        let before = timeline.clone();
        let mut log = CommandLog::new();

        // Shorten the end of `a` by one second: `b` closes the gap.
        log.execute(Box::new(RippleTrimCommand::new(a_id, TrimEdge::Out, -1_000_000)), &mut timeline).unwrap();
        let (_, a) = timeline.find_clip(a_id).unwrap();
        assert_eq!((a.timeline_in, a.timeline_out, a.source_out), (0, 3_000_000, 4_000_000));
        assert_eq!(timeline.find_clip(b_id).unwrap().1.timeline_in, 3_000_000);
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline, before);

        // Trim the start of `a` by one second: it stays at zero, gets shorter, `b` follows.
        log.execute(Box::new(RippleTrimCommand::new(a_id, TrimEdge::In, 1_000_000)), &mut timeline).unwrap();
        let (_, a) = timeline.find_clip(a_id).unwrap();
        assert_eq!((a.timeline_in, a.timeline_out, a.source_in), (0, 3_000_000, 2_000_000));
        assert_eq!(timeline.find_clip(b_id).unwrap().1.timeline_in, 3_000_000);
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline, before);

        // Trimming before the start of the media is refused and changes nothing.
        assert!(log.execute(Box::new(RippleTrimCommand::new(a_id, TrimEdge::In, -2_000_000)), &mut timeline).is_err());
        assert_eq!(timeline, before);
    }

    #[test]
    fn test_edit_clip_and_replace_timeline_undo() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let clip = Clip::new(track_id, Uuid::new_v4(), ClipType::Video, "a", 0, 2_000_000, 0, 2_000_000);
        timeline.tracks[0].clips.push(clip.clone());
        let before = timeline.clone();
        let mut log = CommandLog::new();

        let mut edited = clip.clone();
        edited.properties.fade_in_us = 500_000;
        log.execute(Box::new(EditClipCommand::new("Fade", edited)), &mut timeline).unwrap();
        let (_, c) = timeline.find_clip(clip.id).unwrap();
        assert_eq!(c.fade_factor(0), 0.0);
        assert_eq!(c.fade_factor(250_000), 0.5);
        assert_eq!(c.fade_factor(1_000_000), 1.0);
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline, before);

        let mut other = timeline.clone();
        other.tracks[0].clips.clear();
        log.execute(Box::new(ReplaceTimelineCommand::new("Script", other.clone())), &mut timeline).unwrap();
        assert_eq!(timeline, other);
        log.undo(&mut timeline).unwrap();
        assert_eq!(timeline, before);
        log.redo(&mut timeline).unwrap();
        assert_eq!(timeline, other);
    }
}
