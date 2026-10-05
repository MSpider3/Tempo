use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipType {
    Video,
    Audio,
    Image,
    Title,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    Video,
    Audio,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RationalFps {
    pub num: u32,
    pub den: u32,
}

impl RationalFps {
    pub const FPS_24: Self = Self { num: 24, den: 1 };
    pub const FPS_23_976: Self = Self { num: 24000, den: 1001 };
    pub const FPS_25: Self = Self { num: 25, den: 1 };
    pub const FPS_29_97: Self = Self { num: 30000, den: 1001 };
    pub const FPS_30: Self = Self { num: 30, den: 1 };
    pub const FPS_50: Self = Self { num: 50, den: 1 };
    pub const FPS_59_94: Self = Self { num: 60000, den: 1001 };
    pub const FPS_60: Self = Self { num: 60, den: 1 };

    pub fn to_f64(&self) -> f64 {
        if self.den == 0 {
            30.0
        } else {
            self.num as f64 / self.den as f64
        }
    }

    pub fn frame_duration_us(&self) -> i64 {
        if self.num == 0 {
            33333
        } else {
            (1_000_000 * self.den as i64) / self.num as i64
        }
    }
}

pub fn us_to_frame(us: i64, fps: RationalFps) -> i64 {
    if fps.den == 0 || fps.num == 0 {
        return 0;
    }
    let denom = 1_000_000 * fps.den as i64;
    (us * fps.num as i64 + denom / 2) / denom
}

pub fn frame_to_us(frame: i64, fps: RationalFps) -> i64 {
    if fps.num == 0 {
        return 0;
    }
    let num = frame * 1_000_000 * fps.den as i64;
    (num + fps.num as i64 / 2) / fps.num as i64
}

impl Default for RationalFps {
    fn default() -> Self {
        Self::FPS_30
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub time_us: i64,
    pub property: String,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipProperties {
    pub opacity: f32,
    pub position_x: f32,
    pub position_y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation: f32,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub lift: [f32; 3],
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    pub keyframes: Vec<Keyframe>,
}

impl Default for ClipProperties {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
            volume: 1.0,
            pan: 0.0,
            muted: false,
            lift: [0.0, 0.0, 0.0],
            gamma: [1.0, 1.0, 1.0],
            gain: [1.0, 1.0, 1.0],
            keyframes: Vec::new(),
        }
    }
}

impl ClipProperties {
    pub fn get_base_property(&self, property: &str) -> f32 {
        match property {
            "opacity" => self.opacity,
            "position_x" => self.position_x,
            "position_y" => self.position_y,
            "scale_x" => self.scale_x,
            "scale_y" => self.scale_y,
            "rotation" => self.rotation,
            "volume" => self.volume,
            "pan" => self.pan,
            _ => 1.0,
        }
    }

    pub fn set_base_property(&mut self, property: &str, value: f32) {
        match property {
            "opacity" => self.opacity = value.clamp(0.0, 1.0),
            "position_x" => self.position_x = value,
            "position_y" => self.position_y = value,
            "scale_x" => self.scale_x = value,
            "scale_y" => self.scale_y = value,
            "rotation" => self.rotation = value,
            "volume" => self.volume = value.max(0.0),
            "pan" => self.pan = value.clamp(-1.0, 1.0),
            _ => {}
        }
    }

    pub fn add_keyframe(&mut self, property: impl Into<String>, time_us: i64, value: f32) {
        let prop = property.into();
        self.keyframes.retain(|k| !(k.property == prop && k.time_us == time_us));
        self.keyframes.push(Keyframe {
            time_us,
            property: prop,
            value,
        });
        self.keyframes.sort_by_key(|k| k.time_us);
    }

    pub fn remove_keyframe(&mut self, property: &str, time_us: i64, threshold_us: i64) {
        self.keyframes.retain(|k| !(k.property == property && (k.time_us - time_us).abs() <= threshold_us));
    }

    pub fn evaluate_property(&self, property: &str, time_us: i64) -> f32 {
        let matching: Vec<&Keyframe> = self
            .keyframes
            .iter()
            .filter(|k| k.property == property)
            .collect();

        if matching.is_empty() {
            return self.get_base_property(property);
        }

        if matching.len() == 1 {
            return matching[0].value;
        }

        if time_us <= matching[0].time_us {
            return matching[0].value;
        }

        if time_us >= matching.last().unwrap().time_us {
            return matching.last().unwrap().value;
        }

        for i in 0..matching.len() - 1 {
            let k1 = matching[i];
            let k2 = matching[i + 1];
            if time_us >= k1.time_us && time_us <= k2.time_us {
                let range = k2.time_us - k1.time_us;
                if range == 0 {
                    return k1.value;
                }
                let t = (time_us - k1.time_us) as f64 / range as f64;
                return k1.value + ((k2.value - k1.value) as f64 * t) as f32;
            }
        }

        self.get_base_property(property)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TitleType {
    CenterTitle,
    LowerThird,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TitleData {
    pub title_type: TitleType,
    pub text: String,
    pub subtitle: Option<String>,
    pub font_family: String,
    pub font_size: f32,
    pub font_bold: bool,
    pub font_italic: bool,
    pub color: [u8; 4],
    pub background_color: Option<[u8; 4]>,
    pub background_padding: f32,
    pub custom_position: Option<(f32, f32)>,
}

impl Default for TitleData {
    fn default() -> Self {
        Self {
            title_type: TitleType::CenterTitle,
            text: String::from("Title"),
            subtitle: None,
            font_family: String::from("Sans"),
            font_size: 72.0,
            font_bold: true,
            font_italic: false,
            color: [255, 255, 255, 255],
            background_color: None,
            background_padding: 16.0,
            custom_position: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Uuid,
    pub track_id: Uuid,
    pub source_id: Uuid,
    pub clip_type: ClipType,
    pub name: String,
    pub timeline_in: i64,
    pub timeline_out: i64,
    pub source_in: i64,
    pub source_out: i64,
    pub properties: ClipProperties,
    pub title_data: Option<TitleData>,
}

impl Clip {
    pub fn new(
        track_id: Uuid,
        source_id: Uuid,
        clip_type: ClipType,
        name: impl Into<String>,
        timeline_in: i64,
        timeline_out: i64,
        source_in: i64,
        source_out: i64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            track_id,
            source_id,
            clip_type,
            name: name.into(),
            timeline_in,
            timeline_out,
            source_in,
            source_out,
            properties: ClipProperties::default(),
            title_data: None,
        }
    }

    pub fn duration_us(&self) -> i64 {
        self.timeline_out - self.timeline_in
    }

    pub fn contains_point(&self, time_us: i64) -> bool {
        time_us >= self.timeline_in && time_us < self.timeline_out
    }

    pub fn source_offset_at(&self, timeline_time_us: i64) -> i64 {
        self.source_in + (timeline_time_us - self.timeline_in)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionEdge {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionKind {
    Cut,
    CrossDissolve,
    Crossfade,
    DipToBlack,
    DipToWhite,
    FadeIn,
    FadeOut,
    FadeToBlack,
    FadeToWhite,
    FadeFromBlack,
    FadeFromWhite,
    Plugin(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionAlignment {
    Centered,
    StartAtCut,
    EndAtCut,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub id: Uuid,
    pub clip_id: Uuid,
    pub edge: TransitionEdge,
    pub kind: TransitionKind,
    pub duration_us: i64,
    pub alignment: TransitionAlignment,
    pub plugin_id: Option<String>,
    pub plugin_params: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkerColor {
    Red,
    Green,
    Blue,
    Yellow,
    Orange,
    Purple,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub id: Uuid,
    pub position_us: i64,
    pub name: String,
    pub color: MarkerColor,
    pub duration_us: i64,
    pub note: String,
}

impl Marker {
    pub fn new(position_us: i64, name: impl Into<String>, color: MarkerColor) -> Self {
        Self {
            id: Uuid::new_v4(),
            position_us,
            name: name.into(),
            color,
            duration_us: 0,
            note: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: Uuid,
    pub kind: TrackKind,
    pub kind_index: u32,
    pub name: String,
    pub enabled: bool,
    pub locked: bool,
    pub solo: bool,
    pub volume: f32,
    pub height_px: u32,
    pub color: String,
    pub sort_order: u32,
    pub clips: Vec<Clip>,
    pub transitions: Vec<Transition>,
}

impl Track {
    pub fn new(kind: TrackKind, kind_index: u32, name: impl Into<String>, sort_order: u32) -> Self {
        let default_color = match kind {
            TrackKind::Video => "#5294e2".to_string(),
            TrackKind::Audio => "#2ecc71".to_string(),
        };

        Self {
            id: Uuid::new_v4(),
            kind,
            kind_index,
            name: name.into(),
            enabled: true,
            locked: false,
            solo: false,
            volume: 1.0,
            height_px: 72,
            color: default_color,
            sort_order,
            clips: Vec::new(),
            transitions: Vec::new(),
        }
    }

    pub fn duration_us(&self) -> i64 {
        self.clips
            .iter()
            .map(|c| c.timeline_out)
            .max()
            .unwrap_or(0)
    }

    pub fn sort_clips(&mut self) {
        self.clips.sort_by_key(|c| c.timeline_in);
    }

    pub fn find_clip(&self, clip_id: Uuid) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == clip_id)
    }

    pub fn find_clip_mut(&mut self, clip_id: Uuid) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|c| c.id == clip_id)
    }

    pub fn clip_at(&self, position_us: i64) -> Option<&Clip> {
        self.clips.iter().find(|c| c.contains_point(position_us))
    }

    pub fn has_collision(&self, start_us: i64, end_us: i64, ignore_clip_id: Option<Uuid>) -> bool {
        for clip in &self.clips {
            if let Some(ignored) = ignore_clip_id {
                if clip.id == ignored {
                    continue;
                }
            }
            if start_us < clip.timeline_out && end_us > clip.timeline_in {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new_default()
    }
}

impl Timeline {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            markers: Vec::new(),
        }
    }

    pub fn new_default() -> Self {
        Self {
            tracks: vec![
                Track::new(TrackKind::Video, 1, "Video 1", 0),
                Track::new(TrackKind::Video, 2, "Video 2", 1),
                Track::new(TrackKind::Audio, 1, "Audio 1", 2),
                Track::new(TrackKind::Audio, 2, "Audio 2", 3),
            ],
            markers: Vec::new(),
        }
    }

    pub fn duration_us(&self) -> i64 {
        self.tracks
            .iter()
            .filter(|t| t.enabled)
            .map(|t| t.duration_us())
            .max()
            .unwrap_or(0)
    }

    pub fn find_track(&self, track_id: Uuid) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == track_id)
    }

    pub fn find_track_mut(&mut self, track_id: Uuid) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == track_id)
    }

    pub fn find_clip(&self, clip_id: Uuid) -> Option<(&Track, &Clip)> {
        for track in &self.tracks {
            if let Some(clip) = track.find_clip(clip_id) {
                return Some((track, clip));
            }
        }
        None
    }

    pub fn find_clip_mut(&mut self, clip_id: Uuid) -> Option<&mut Clip> {
        for track in &mut self.tracks {
            if let Some(clip) = track.find_clip_mut(clip_id) {
                return Some(clip);
            }
        }
        None
    }

    pub fn clips_at_time(&self, time_us: i64) -> Vec<&Clip> {
        let mut result = Vec::new();
        for track in &self.tracks {
            if !track.enabled {
                continue;
            }
            for clip in &track.clips {
                if clip.contains_point(time_us) {
                    result.push(clip);
                }
            }
        }
        result
    }

    pub fn clips_at(&self, position_us: i64) -> Vec<(&Track, &Clip)> {
        let mut result = Vec::new();
        for track in &self.tracks {
            if track.enabled {
                if let Some(clip) = track.clip_at(position_us) {
                    result.push((track, clip));
                }
            }
        }
        result
    }

    pub fn find_snap_point(&self, target_us: i64, threshold_us: i64, ignore_clip_id: Option<Uuid>) -> Option<i64> {
        let mut best_snap: Option<i64> = None;
        let mut best_diff = threshold_us + 1;

        let mut check_point = |pt: i64| {
            let diff = (pt - target_us).abs();
            if diff <= threshold_us && diff < best_diff {
                best_diff = diff;
                best_snap = Some(pt);
            }
        };

        // Snap to zero
        check_point(0);

        // Snap to markers
        for marker in &self.markers {
            check_point(marker.position_us);
        }

        // Snap to clip in and out points
        for track in &self.tracks {
            for clip in &track.clips {
                if let Some(ignored) = ignore_clip_id {
                    if clip.id == ignored {
                        continue;
                    }
                }
                check_point(clip.timeline_in);
                check_point(clip.timeline_out);
            }
        }

        best_snap
    }

    pub fn add_marker(&mut self, marker: Marker) {
        self.markers.push(marker);
        self.markers.sort_by_key(|m| m.position_us);
    }

    pub fn remove_marker(&mut self, marker_id: Uuid) -> Option<Marker> {
        if let Some(idx) = self.markers.iter().position(|m| m.id == marker_id) {
            Some(self.markers.remove(idx))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaSource {
    pub id: Uuid,
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub media_type: MediaType,
    pub duration_us: i64,
    pub is_missing: bool,
    pub video_width: Option<u32>,
    pub video_height: Option<u32>,
    pub video_fps: Option<RationalFps>,
    pub video_codec: Option<String>,
    pub video_color_range: Option<u8>,
    pub video_color_space: Option<u8>,
    pub video_bit_depth: Option<u8>,
    pub video_has_alpha: bool,
    pub audio_sample_rate: Option<u32>,
    pub audio_channels: Option<u32>,
    pub audio_codec: Option<String>,
    pub audio_bit_rate: Option<u32>,
    pub proxy_path: Option<PathBuf>,
    pub proxy_ready: bool,
    pub thumbnail_data: Option<Vec<u8>>,
    pub imported_at: i64,
    pub import_order: u32,
}

impl MediaSource {
    pub fn new(path: PathBuf, media_type: MediaType, duration_us: i64) -> Self {
        Self {
            id: Uuid::new_v4(),
            relative_path: path.clone(),
            path,
            media_type,
            duration_us,
            is_missing: false,
            video_width: None,
            video_height: None,
            video_fps: None,
            video_codec: None,
            video_color_range: None,
            video_color_space: None,
            video_bit_depth: None,
            video_has_alpha: false,
            audio_sample_rate: None,
            audio_channels: None,
            audio_codec: None,
            audio_bit_rate: None,
            proxy_path: None,
            proxy_ready: false,
            thumbnail_data: None,
            imported_at: 0,
            import_order: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiState {
    pub active_page: String,
    pub timeline_zoom: f32,
    pub timeline_scroll_x: i64,
    pub left_panel_width: i32,
    pub left_panel_tab: String,
    pub inspector_open: bool,
    pub viewer_split: f32,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            active_page: "cut".to_string(),
            timeline_zoom: 1.0,
            timeline_scroll_x: 0,
            left_panel_width: 300,
            left_panel_tab: "media".to_string(),
            inspector_open: true,
            viewer_split: 0.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub created_at: i64,
    pub modified_at: i64,
    pub width: u32,
    pub height: u32,
    pub fps: RationalFps,
    pub sample_rate: u32,
    pub channels: u32,
    pub proxy_dir: PathBuf,
    pub timeline: Timeline,
    pub sources: HashMap<Uuid, MediaSource>,
    pub ui_state: UiState,
}

impl Project {
    pub fn new(name: impl Into<String>, width: u32, height: u32, fps: RationalFps) -> Self {
        let now = 0; // will be populated with unix us timestamp
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            created_at: now,
            modified_at: now,
            width,
            height,
            fps,
            sample_rate: 48000,
            channels: 2,
            proxy_dir: PathBuf::from("/tmp/tempo_proxies"),
            timeline: Timeline::new_default(),
            sources: HashMap::new(),
            ui_state: UiState::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rational_fps() {
        let fps24 = RationalFps::FPS_24;
        assert_eq!(fps24.to_f64(), 24.0);
        assert_eq!(fps24.frame_duration_us(), 41666);

        let fps30 = RationalFps::FPS_30;
        assert_eq!(fps30.to_f64(), 30.0);
        assert_eq!(fps30.frame_duration_us(), 33333);
    }

    #[test]
    fn test_us_to_frame_30fps() {
        let fps = RationalFps::FPS_30;
        assert_eq!(us_to_frame(0, fps), 0);
        assert_eq!(us_to_frame(16_000, fps), 0); // < half frame
        assert_eq!(us_to_frame(33_333, fps), 1); // 1 frame
        assert_eq!(us_to_frame(1_000_000, fps), 30);
        assert_eq!(us_to_frame(2_000_000, fps), 60);
    }

    #[test]
    fn test_us_to_frame_2397fps() {
        let fps = RationalFps::FPS_23_976;
        let frame_duration = fps.frame_duration_us();
        assert_eq!(us_to_frame(0, fps), 0);
        assert_eq!(us_to_frame(frame_duration * 24, fps), 24);
    }

    #[test]
    fn test_frame_to_us_round_trip() {
        let fps = RationalFps::FPS_30;
        for frame in 0..100 {
            let us = frame_to_us(frame, fps);
            let round_frame = us_to_frame(us, fps);
            assert_eq!(round_frame, frame);
        }
    }

    #[test]
    fn test_clip_duration() {
        let clip = Clip::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            ClipType::Video,
            "c",
            1_000_000,
            4_500_000,
            500_000,
            4_000_000,
        );
        assert_eq!(clip.duration_us(), 3_500_000);
    }

    #[test]
    fn test_clip_source_offset_at() {
        let clip = Clip::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            ClipType::Video,
            "c",
            2_000_000,
            5_000_000,
            1_000_000,
            4_000_000,
        );
        assert_eq!(clip.source_offset_at(2_000_000), 1_000_000);
        assert_eq!(clip.source_offset_at(3_000_000), 2_000_000);
        assert_eq!(clip.source_offset_at(4_500_000), 3_500_000);
    }

    #[test]
    fn test_clip_invariant_source_out_minus_in_equals_timeline_duration() {
        let clip = Clip::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            ClipType::Video,
            "test_inv",
            1_000_000,
            6_000_000,
            2_000_000,
            7_000_000,
        );
        assert_eq!(
            clip.source_out - clip.source_in,
            clip.timeline_out - clip.timeline_in
        );
    }

    #[test]
    fn test_timeline_clips_at_time_empty() {
        let timeline = Timeline::new_default();
        let clips = timeline.clips_at_time(1_000_000);
        assert!(clips.is_empty());
    }

    #[test]
    fn test_timeline_clips_at_time_single_clip() {
        let mut timeline = Timeline::new_default();
        let clip = Clip::new(
            timeline.tracks[0].id,
            Uuid::new_v4(),
            ClipType::Video,
            "v1",
            1_000_000,
            3_000_000,
            0,
            2_000_000,
        );
        timeline.tracks[0].clips.push(clip);

        assert_eq!(timeline.clips_at_time(500_000).len(), 0);
        assert_eq!(timeline.clips_at_time(1_500_000).len(), 1);
        assert_eq!(timeline.clips_at_time(3_500_000).len(), 0);
    }

    #[test]
    fn test_timeline_clips_at_time_multiple_tracks() {
        let mut timeline = Timeline::new_default();
        let c1 = Clip::new(
            timeline.tracks[0].id,
            Uuid::new_v4(),
            ClipType::Video,
            "v1",
            1_000_000,
            4_000_000,
            0,
            3_000_000,
        );
        let c2 = Clip::new(
            timeline.tracks[1].id,
            Uuid::new_v4(),
            ClipType::Video,
            "v2",
            2_000_000,
            5_000_000,
            0,
            3_000_000,
        );
        timeline.tracks[0].clips.push(c1);
        timeline.tracks[1].clips.push(c2);

        assert_eq!(timeline.clips_at_time(1_500_000).len(), 1);
        assert_eq!(timeline.clips_at_time(2_500_000).len(), 2);
        assert_eq!(timeline.clips_at_time(4_500_000).len(), 1);
    }

    #[test]
    fn test_timeline_clips_at_time_at_edit_point() {
        let mut timeline = Timeline::new_default();
        let c1 = Clip::new(
            timeline.tracks[0].id,
            Uuid::new_v4(),
            ClipType::Video,
            "c1",
            0,
            2_000_000,
            0,
            2_000_000,
        );
        let c2 = Clip::new(
            timeline.tracks[0].id,
            Uuid::new_v4(),
            ClipType::Video,
            "c2",
            2_000_000,
            4_000_000,
            0,
            2_000_000,
        );
        timeline.tracks[0].clips.push(c1);
        timeline.tracks[0].clips.push(c2);

        // At exact boundary 2_000_000, c1 has out=2_000_000 ([0, 2_000_000)) so c1 does not contain 2_000_000,
        // and c2 has in=2_000_000 ([2_000_000, 4_000_000)) so c2 contains 2_000_000!
        let at_cut = timeline.clips_at_time(2_000_000);
        assert_eq!(at_cut.len(), 1);
        assert_eq!(at_cut[0].name, "c2");
    }

    #[test]
    fn test_timeline_duration_recomputed_on_clip_change() {
        let mut timeline = Timeline::new_default();
        assert_eq!(timeline.duration_us(), 0);

        let clip = Clip::new(
            timeline.tracks[0].id,
            Uuid::new_v4(),
            ClipType::Video,
            "c1",
            1_000_000,
            6_000_000,
            0,
            5_000_000,
        );
        timeline.tracks[0].clips.push(clip);
        assert_eq!(timeline.duration_us(), 6_000_000);

        timeline.tracks[0].clips[0].timeline_out = 8_000_000;
        assert_eq!(timeline.duration_us(), 8_000_000);
    }

    #[test]
    fn test_clip_contains_point_and_duration() {
        let clip = Clip::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            ClipType::Video,
            "test",
            1_000_000,
            4_000_000,
            0,
            3_000_000,
        );
        assert_eq!(clip.duration_us(), 3_000_000);
        assert!(!clip.contains_point(999_999));
        assert!(clip.contains_point(1_000_000));
        assert!(clip.contains_point(2_500_000));
        assert!(clip.contains_point(3_999_999));
        assert!(!clip.contains_point(4_000_000));
    }

    #[test]
    fn test_timeline_snapping() {
        let mut timeline = Timeline::new_default();
        let track_id = timeline.tracks[0].id;
        let clip = Clip::new(
            track_id,
            Uuid::new_v4(),
            ClipType::Video,
            "c1",
            2_000_000,
            5_000_000,
            0,
            3_000_000,
        );
        timeline.tracks[0].clips.push(clip);

        let marker = Marker::new(8_000_000, "M1", MarkerColor::Blue);
        timeline.add_marker(marker);

        let threshold = 50_000; // 50ms

        // Close to 0
        assert_eq!(timeline.find_snap_point(30_000, threshold, None), Some(0));

        // Close to clip start (2_000_000)
        assert_eq!(timeline.find_snap_point(1_980_000, threshold, None), Some(2_000_000));

        // Close to clip end (5_000_000)
        assert_eq!(timeline.find_snap_point(5_020_000, threshold, None), Some(5_000_000));

        // Close to marker (8_000_000)
        assert_eq!(timeline.find_snap_point(7_990_000, threshold, None), Some(8_000_000));

        // Far away from any snap point
        assert_eq!(timeline.find_snap_point(6_000_000, threshold, None), None);
    }

    #[test]
    fn test_keyframe_interpolation() {
        let mut props = ClipProperties::default();
        props.opacity = 0.8;
        assert_eq!(props.evaluate_property("opacity", 1_000_000), 0.8);

        // Add 2 keyframes: fade in from 0.0 at 1s to 1.0 at 3s
        props.add_keyframe("opacity", 1_000_000, 0.0);
        props.add_keyframe("opacity", 3_000_000, 1.0);

        // Before first keyframe: clamped to first value
        assert_eq!(props.evaluate_property("opacity", 0), 0.0);
        assert_eq!(props.evaluate_property("opacity", 1_000_000), 0.0);

        // Midpoint at 2s: 0.5
        let mid = props.evaluate_property("opacity", 2_000_000);
        assert!((mid - 0.5).abs() < 1e-4);

        // Quarter at 1.5s: 0.25
        let qtr = props.evaluate_property("opacity", 1_500_000);
        assert!((qtr - 0.25).abs() < 1e-4);

        // Three-quarters at 2.5s: 0.75
        let tqtr = props.evaluate_property("opacity", 2_500_000);
        assert!((tqtr - 0.75).abs() < 1e-4);

        // Exactly at second keyframe & after
        assert_eq!(props.evaluate_property("opacity", 3_000_000), 1.0);
        assert_eq!(props.evaluate_property("opacity", 5_000_000), 1.0);

        // Another property remains untouched
        assert_eq!(props.evaluate_property("scale_x", 2_000_000), 1.0);
    }
}
