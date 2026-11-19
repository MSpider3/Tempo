//! Audio transitions as gain ramps, shared by the live player and the export.
//!
//! A transition on an audio track ramps the outgoing clip down and the incoming clip up
//! over `Track::transition_span`, the incoming side using handle footage before the cut
//! and the outgoing side its footage after it (extra "pieces" in separate strips, since
//! they overlap the clips). A cut is equal power (sin/cos, Resolve's "Cross Fade +3 dB");
//! a head or tail transition is a plain linear fade. Video transitions never touch sound.

use std::collections::HashMap;

use tempo_timeline::{Clip, ClipType, MediaSource, Track, TransitionEdge, TransitionKind};
use uuid::Uuid;

/// A gain ramp over `[start_us, end_us]`, outside which it is 0 or 1 to match its end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ramp {
    pub start_us: i64,
    pub end_us: i64,
    /// Rising (0 to 1) for the incoming side, falling for the outgoing one.
    pub rising: bool,
    pub equal_power: bool,
}

impl Ramp {
    /// Progress through the ramp for the gain of the side it applies to, 0.0 to 1.0.
    pub fn level(&self, p: f64) -> f64 {
        let x = if self.rising { p } else { 1.0 - p };
        if self.equal_power {
            (x * std::f64::consts::FRAC_PI_2).sin()
        } else {
            x
        }
    }

    pub fn gain_at(&self, time_us: i64) -> f32 {
        let span = (self.end_us - self.start_us).max(1) as f64;
        self.level(((time_us - self.start_us) as f64 / span).clamp(0.0, 1.0)) as f32
    }
}

/// One audio track as strips of clips (each strip plays at most one clip at a time) and
/// the ramps to apply to clips by id.
#[derive(Debug, Clone, Default)]
pub struct AudioStrips {
    pub strips: Vec<Vec<Clip>>,
    pub ramps: HashMap<Uuid, Vec<Ramp>>,
}

impl AudioStrips {
    /// The product of the ramps on a clip at a time.
    pub fn gain(ramps: &HashMap<Uuid, Vec<Ramp>>, clip: &Clip, time_us: i64) -> f32 {
        ramps.get(&clip.id).map_or(1.0, |rs| rs.iter().map(|r| r.gain_at(time_us)).product())
    }
}

fn head_room(clip: &Clip) -> i64 {
    clip.to_timeline_floor_us(clip.source_in).max(0)
}

fn tail_room(clip: &Clip, sources: &HashMap<Uuid, MediaSource>) -> i64 {
    sources.get(&clip.source_id).map_or(i64::MAX, |s| clip.to_timeline_floor_us((s.duration_us - clip.source_out).max(0)))
}

/// A copy of `clip` playing `[from, to)` of the timeline, from `source_in`.
fn piece(clip: &Clip, from: i64, to: i64, source_in: i64) -> Clip {
    let mut p = clip.clone();
    p.id = Uuid::new_v4();
    p.timeline_in = from;
    p.timeline_out = to;
    p.source_in = source_in;
    p.source_out = source_in + clip.to_source_us(to - from);
    p.properties.fade_in_us = 0;
    p.properties.fade_out_us = 0;
    p.properties.dissolve_in_us = 0;
    p
}

/// Builds the strips and ramps for an audio track. Strip 0 is the track's clips; the
/// next two hold the handle pieces before and after the cuts. A cut's ramps cover only the
/// footage there is (before the cut: the incoming clip's head, after it: the outgoing
/// clip's tail), each side also limited to the neighbouring clip's length so pieces in one
/// strip never overlap; with no footage on either side it is a plain cut.
pub fn audio_strips(track: &Track, sources: &HashMap<Uuid, MediaSource>) -> AudioStrips {
    let mut out = AudioStrips { strips: vec![track.clips.clone(), Vec::new(), Vec::new()], ramps: HashMap::new() };
    let mut ramp = |id: Uuid, r: Ramp| out.ramps.entry(id).or_default().push(r);
    let (mut heads, mut tails) = (Vec::new(), Vec::new());
    for t in track.transitions.iter().filter(|t| t.kind != TransitionKind::Cut) {
        let Some(clip) = track.find_clip(t.clip_id) else { continue };
        let Some((start_us, end_us)) = track.transition_span(t) else { continue };
        if end_us <= start_us {
            continue;
        }
        let outgoing = (t.edge == TransitionEdge::In).then(|| track.clips.iter().find(|c| c.id != clip.id && c.timeline_out == clip.timeline_in)).flatten();
        let Some(outgoing) = outgoing else {
            let r = |rising| Ramp { start_us, end_us, rising, equal_power: false };
            ramp(clip.id, r(t.edge == TransitionEdge::In));
            continue;
        };
        let cut = clip.timeline_in;
        let before = (cut - start_us).min(head_room(clip)).min(outgoing.duration_us()).max(0);
        let after = (end_us - cut).min(tail_room(outgoing, sources)).min(clip.duration_us()).max(0);
        if before + after == 0 {
            continue;
        }
        let r = |rising| Ramp { start_us: cut - before, end_us: cut + after, rising, equal_power: true };
        ramp(outgoing.id, r(false));
        ramp(clip.id, r(true));
        if before > 0 && clip.clip_type == ClipType::Audio {
            let p = piece(clip, cut - before, cut, clip.source_in - clip.to_source_us(before));
            ramp(p.id, r(true));
            heads.push(p);
        }
        if after > 0 && outgoing.clip_type == ClipType::Audio {
            let p = piece(outgoing, cut, cut + after, outgoing.source_out);
            ramp(p.id, r(false));
            tails.push(p);
        }
    }
    out.strips[1] = heads;
    out.strips[2] = tails;
    out.strips.retain(|s| !s.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::{TransitionEdge, TransitionKind};

    #[test]
    fn equal_power_ramps_keep_constant_power() {
        let (up, down) = (Ramp { start_us: 0, end_us: 100, rising: true, equal_power: true }, Ramp { rising: false, ..Ramp { start_us: 0, end_us: 100, rising: true, equal_power: true } });
        for t in [0, 25, 50, 75, 100] {
            let (a, b) = (up.gain_at(t), down.gain_at(t));
            assert!((a * a + b * b - 1.0).abs() < 1e-5, "{t}: {a} {b}");
        }
        assert_eq!(down.gain_at(-5), 1.0);
        assert_eq!(up.gain_at(500), 1.0);
    }

    /// A[0,4s] and B[4,8s] where B has no footage before its in-point: a centred 1 s
    /// cross fade can only use A's tail, so the fade shrinks to [4s,4.5s] and the summed power
    /// stays level with no step at the cut.
    #[test]
    fn cross_fade_without_a_head_handle_keeps_power_level() {
        use tempo_timeline::{MediaType, Project, RationalFps, Transition, TransitionAlignment, TrackKind};
        let mut project = Project::new("t", 320, 180, RationalFps::FPS_30);
        let source = MediaSource::new(std::env::temp_dir().join("x.wav"), MediaType::Audio, 8_000_000);
        let a = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio).map(|t| t.id).unwrap();
        let track = project.timeline.find_track_mut(a).unwrap();
        track.clips.push(Clip::new(a, source.id, ClipType::Audio, "a", 0, 4_000_000, 1_000_000, 5_000_000));
        track.clips.push(Clip::new(a, source.id, ClipType::Audio, "b", 4_000_000, 8_000_000, 0, 4_000_000));
        let b = track.clips[1].id;
        track.transitions.push(Transition::new(b, TransitionEdge::In, TransitionKind::Crossfade, 1_000_000, TransitionAlignment::Centered));
        let sources = HashMap::from([(source.id, source)]);
        let strips = audio_strips(project.timeline.find_track_mut(a).unwrap(), &sources);
        let power = |t: i64| -> f32 {
            strips.strips.iter().flatten().filter(|c| c.timeline_in <= t && t < c.timeline_out).map(|c| AudioStrips::gain(&strips.ramps, c, t).powi(2)).sum()
        };
        for t in (3_000_000..5_500_000).step_by(50_000) {
            assert!((0.9..=1.1).contains(&power(t)), "{t}: {}", power(t));
        }
        assert!((power(3_999_999) - power(4_000_000)).abs() < 0.05, "{} vs {}", power(3_999_999), power(4_000_000));
    }
}
