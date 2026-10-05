use std::path::PathBuf;
use tempo_project::{create_new_project, save_project};
use tempo_timeline::types::{
    Clip, ClipType, MediaSource, MediaType, RationalFps, Track, TrackKind,
};
use uuid::Uuid;

fn main() {
    let out_dir = PathBuf::from("tests/projects");
    std::fs::create_dir_all(&out_dir).expect("Failed to create tests/projects directory");

    let media_path = PathBuf::from("tests/media/sample_1080p_h264.mp4")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("/run/media/mehulgolecha/Extra Volume/Projects/Tempo (Video Editor)/tests/media/sample_1080p_h264.mp4"));

    // 1. Generate basic_cut.tempo (3 clips for export testing)
    let basic_cut_path = out_dir.join("basic_cut.tempo");
    let mut basic_cut = create_new_project("Basic Cut", 1920, 1080, RationalFps::FPS_24, &basic_cut_path)
        .expect("Failed to create basic_cut project");

    let mut source = MediaSource::new(media_path.clone(), MediaType::Video, 10_000_000);
    source.video_width = Some(1920);
    source.video_height = Some(1080);
    source.video_fps = Some(RationalFps::FPS_24);
    let source_id = source.id;
    basic_cut.sources.insert(source_id, source);

    if let Some(track) = basic_cut.timeline.tracks.iter_mut().find(|t| t.kind == TrackKind::Video) {
        let track_id = track.id;
        // Add 3 consecutive clips
        track.clips.push(Clip::new(
            track_id,
            source_id,
            ClipType::Video,
            "Clip 1",
            0,
            2_000_000,
            0,
            2_000_000,
        ));
        track.clips.push(Clip::new(
            track_id,
            source_id,
            ClipType::Video,
            "Clip 2",
            2_000_000,
            4_000_000,
            0,
            2_000_000,
        ));
        track.clips.push(Clip::new(
            track_id,
            source_id,
            ClipType::Video,
            "Clip 3",
            4_000_000,
            6_000_000,
            0,
            2_000_000,
        ));
    }

    save_project(&basic_cut, &basic_cut_path).expect("Failed to save basic_cut.tempo");
    println!("Generated {:?}", basic_cut_path);

    // 2. Generate multitrack.tempo (2 video + 2 audio tracks)
    let multitrack_path = out_dir.join("multitrack.tempo");
    let mut multitrack = create_new_project("Multitrack", 1920, 1080, RationalFps::FPS_24, &multitrack_path)
        .expect("Failed to create multitrack project");

    multitrack.sources.insert(source_id, basic_cut.sources[&source_id].clone());

    // Ensure tracks: V1, V2, A1, A2
    multitrack.timeline.tracks.clear();
    let mut v1 = Track::new(TrackKind::Video, 1, "V1", 0);
    let v1_id = v1.id;
    v1.clips.push(Clip::new(v1_id, source_id, ClipType::Video, "Base Video", 0, 5_000_000, 0, 5_000_000));

    let mut v2 = Track::new(TrackKind::Video, 2, "V2", 1);
    let v2_id = v2.id;
    v2.clips.push(Clip::new(v2_id, source_id, ClipType::Video, "Overlay Video", 1_000_000, 4_000_000, 1_000_000, 4_000_000));

    let mut a1 = Track::new(TrackKind::Audio, 1, "A1", 2);
    let a1_id = a1.id;
    a1.clips.push(Clip::new(a1_id, source_id, ClipType::Audio, "Main Dialogue", 0, 5_000_000, 0, 5_000_000));

    let mut a2 = Track::new(TrackKind::Audio, 2, "A2", 3);
    let a2_id = a2.id;
    a2.clips.push(Clip::new(a2_id, source_id, ClipType::Audio, "Background Music", 0, 5_000_000, 0, 5_000_000));

    multitrack.timeline.tracks.push(v1);
    multitrack.timeline.tracks.push(v2);
    multitrack.timeline.tracks.push(a1);
    multitrack.timeline.tracks.push(a2);

    save_project(&multitrack, &multitrack_path).expect("Failed to save multitrack.tempo");
    println!("Generated {:?}", multitrack_path);
}
