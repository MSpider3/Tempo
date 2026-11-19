//! Writes the project the end-to-end tests open (see docs/TESTING_E2E.md):
//!   cargo run -p tempo-app --example e2e_fixture -- out.tempo a.mp4 b.mp4 c.mp4
//! One timeline, 1920x1080 at 30 fps, with only V1 and A1. Each file is placed back to
//! back from 0 without its first and last second, so every cut has handles for a centred
//! transition; picture and sound are linked as when dragged in by hand. No markers.

use std::path::PathBuf;

use tempo_media::MediaEngine;
use tempo_timeline::{Clip, ClipType, Project, RationalFps, TrackKind};

/// Media left unused before and after each clip.
const HANDLE_US: i64 = 1_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().ok_or("usage: e2e_fixture <out.tempo> <media>...")?);
    let files: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if files.is_empty() {
        return Err("give at least one media file".into());
    }
    tempo_media::ensure_ffmpeg_init();

    let mut project = Project::new("E2E Project", 1920, 1080, RationalFps::FPS_30);
    // Only V1 and A1, so the tests can make V2/A2 through the ghost band.
    project.timeline.tracks.retain(|t| t.kind_index == 1);
    let track = |kind| project.timeline.tracks.iter().find(|t| t.kind == kind).map(|t| t.id);
    let (v1, a1) = (track(TrackKind::Video).ok_or("no V1")?, track(TrackKind::Audio).ok_or("no A1")?);

    let mut at = 0i64;
    for (i, path) in files.iter().enumerate() {
        let mut source = MediaEngine::create_media_source(path)?;
        source.import_order = i as u32;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let len = source.duration_us - 2 * HANDLE_US;
        if len <= 0 {
            return Err(format!("{} is too short: it needs more than two seconds", path.display()).into());
        }
        let link = source.audio_channels.is_some().then(uuid::Uuid::new_v4);
        let mut place = |track, kind| -> Result<(), Box<dyn std::error::Error>> {
            let mut clip = Clip::new(track, source.id, kind, name.clone(), at, at + len, HANDLE_US, HANDLE_US + len);
            clip.properties.link = link;
            project.timeline.find_track_mut(track).ok_or("track")?.clips.push(clip);
            Ok(())
        };
        place(v1, ClipType::Video)?;
        if link.is_some() {
            place(a1, ClipType::Audio)?;
        }
        project.sources.insert(source.id, source);
        at += len;
    }

    tempo_project::save_project(&project, &out)?;
    println!("wrote {}", out.display());
    Ok(())
}
