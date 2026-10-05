//! Builds a small project from the sample media in tests/media, for manual
//! checks and screenshots:  cargo run -p tempo-app --example make_demo_project -- out.tempo
//! Extra arguments are media files to use instead of the samples; each is placed whole.

use std::path::PathBuf;

use tempo_media::MediaEngine;
use tempo_timeline::{Clip, ClipType, Marker, MarkerColor, Project, RationalFps, TrackKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("usage: make_demo_project <out.tempo>")?);
    let media = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/media");
    tempo_media::ensure_ffmpeg_init();

    let mut project = Project::new("Demo Project", 1920, 1080, RationalFps::FPS_30);
    let track = |p: &Project, kind: TrackKind, index: u32| {
        p.timeline.tracks.iter().find(|t| t.kind == kind && t.kind_index == index).map(|t| t.id)
    };
    let (v1, v2, a1) = (
        track(&project, TrackKind::Video, 1).ok_or("no V1")?,
        track(&project, TrackKind::Video, 2).ok_or("no V2")?,
        track(&project, TrackKind::Audio, 1).ok_or("no A1")?,
    );

    let mut at = 0i64;
    let own: Vec<PathBuf> = std::env::args().skip(2).map(PathBuf::from).collect();
    let files: Vec<PathBuf> = if own.is_empty() {
        ["sample_1080p_h264.mp4", "sample_10s_sync.mp4", "sample_720p_vp9.webm"].iter().map(|f| media.join(f)).collect()
    } else {
        own.clone()
    };
    for (i, path) in files.iter().enumerate() {
        let started = std::time::Instant::now();
        let mut source = MediaEngine::create_media_source(path)?;
        println!("probed {} in {:?}", path.display(), started.elapsed());
        let file = &path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        source.import_order = i as u32;
        let len = if own.is_empty() { source.duration_us.min(4_000_000) } else { source.duration_us };
        let add = |p: &mut Project, t, kind| -> Result<(), Box<dyn std::error::Error>> {
            let clip = Clip::new(t, source.id, kind, file.clone(), at, at + len, 0, len);
            p.timeline.find_track_mut(t).ok_or("track")?.clips.push(clip);
            Ok(())
        };
        add(&mut project, v1, ClipType::Video)?;
        if source.audio_channels.is_some() {
            add(&mut project, a1, ClipType::Audio)?;
        }
        if i == 1 && own.is_empty() {
            // A short overlay on V2.
            let clip = Clip::new(v2, source.id, ClipType::Video, "overlay", at + 500_000, at + 2_000_000, 0, 1_500_000);
            project.timeline.find_track_mut(v2).ok_or("track")?.clips.push(clip);
        }
        project.sources.insert(source.id, source);
        at += len;
    }
    for (sec, name, color) in [(0, "Intro", MarkerColor::Blue), (4, "Second clip", MarkerColor::Green), (9, "", MarkerColor::Red)] {
        project.timeline.add_marker(Marker::new(sec * 1_000_000, name, color));
    }

    tempo_project::save_project(&project, &out)?;
    println!("wrote {}", out.display());
    Ok(())
}
