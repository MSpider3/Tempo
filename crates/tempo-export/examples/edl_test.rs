use std::path::PathBuf;
use tempo_export::edl::export_timeline_to_edl;
use tempo_timeline::types::{Clip, ClipType, RationalFps, Timeline, Track, TrackKind};
use uuid::Uuid;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut project_path = None;
    let mut output_path = None;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--output" || args[i] == "-o" {
            if i + 1 < args.len() {
                output_path = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
        } else if !args[i].starts_with('-') && project_path.is_none() {
            project_path = Some(PathBuf::from(&args[i]));
        }
        i += 1;
    }

    let out_file = output_path.unwrap_or_else(|| PathBuf::from("/tmp/test_export.edl"));
    println!("EDL Output path: {:?}", out_file);

    let (timeline, title) = if let Some(ref path) = project_path {
        if path.exists() {
            match tempo_project::load_project(path) {
                Ok(proj) => {
                    println!("Loaded project: {} from {:?}", proj.name, path);
                    (proj.timeline, proj.name)
                }
                Err(e) => {
                    eprintln!("Warning: failed to load project ({}), creating mock timeline", e);
                    create_mock_timeline()
                }
            }
        } else {
            eprintln!("Project {:?} does not exist, creating mock timeline", path);
            create_mock_timeline()
        }
    } else {
        create_mock_timeline()
    };

    let edl_content = export_timeline_to_edl(&timeline, &title, RationalFps::FPS_30);
    std::fs::write(&out_file, &edl_content).expect("Write EDL file");

    println!("✓ Successfully exported CMX 3600 EDL ({} bytes)", edl_content.len());
    println!("--- EDL Preview ---");
    for line in edl_content.lines().take(12) {
        println!("{}", line);
    }
    println!("-------------------");
}

fn create_mock_timeline() -> (Timeline, String) {
    let mut timeline = Timeline::new();
    let v1 = Track::new(TrackKind::Video, 0, "V1", 0);
    let mut v1 = v1;

    let clip1 = Clip::new(
        v1.id,
        Uuid::new_v4(),
        ClipType::Video,
        "sample_video.mp4",
        0,
        5_000_000,
        0,
        5_000_000,
    );
    let clip2 = Clip::new(
        v1.id,
        Uuid::new_v4(),
        ClipType::Video,
        "clip_b.mp4",
        5_000_000,
        10_000_000,
        1_000_000,
        6_000_000,
    );

    v1.clips.push(clip1);
    v1.clips.push(clip2);
    timeline.tracks.push(v1);

    (timeline, "Multitrack Test".to_string())
}
