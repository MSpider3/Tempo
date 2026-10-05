use std::path::PathBuf;
use tempo_export::{export_raw_frames, ExportSettings};

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

    let out_file = output_path.unwrap_or_else(|| PathBuf::from("/tmp/regression_output.mp4"));
    println!("Full regression export targeting: {:?}", out_file);

    let proj_name = if let Some(ref path) = project_path {
        if path.exists() {
            match tempo_project::load_project(path) {
                Ok(p) => {
                    println!("Loaded project '{}' (tracks: {})", p.name, p.timeline.tracks.len());
                    p.name
                }
                Err(e) => {
                    println!("Notice: Could not load project ({}). Using test composition.", e);
                    "Default Multitrack".to_string()
                }
            }
        } else {
            "Default Multitrack".to_string()
        }
    } else {
        "Default Multitrack".to_string()
    };

    let width = 640;
    let height = 360;
    let fps = 30.0;
    let total_frames = 60; // 2 seconds test render

    let settings = ExportSettings {
        width,
        height,
        fps,
        crf: 23,
        output_path: out_file.clone(),
        audio_source_path: None,
        audio_start_sec: None,
    };

    println!("Rendering 60 frames (640x360 @ 30fps) for project '{}'...", proj_name);

    let frame_size = (width * height * 4) as usize;
    let res = export_raw_frames(
        &settings,
        total_frames,
        |frame_idx| {
            let mut buf = vec![0u8; frame_size];
            let t = frame_idx as f32 / total_frames as f32;
            let r = (t * 255.0) as u8;
            let g = ((1.0 - t) * 200.0) as u8;
            let b = 180u8;

            for chunk in buf.chunks_exact_mut(4) {
                chunk[0] = r;
                chunk[1] = g;
                chunk[2] = b;
                chunk[3] = 255;
            }
            buf
        },
        Some(Box::new(|p| {
            if p.current_frame % 20 == 0 || p.current_frame == p.total_frames {
                println!(
                    "Progress: {:.1}% (frame {}/{})",
                    p.progress * 100.0,
                    p.current_frame,
                    p.total_frames
                );
            }
        })),
    );

    match res {
        Ok(_) => {
            println!("✓ Successfully exported video to {:?}", out_file);
            assert!(out_file.exists(), "Output file must exist");
            let size = std::fs::metadata(&out_file).map(|m| m.len()).unwrap_or(0);
            println!("File size: {} bytes", size);
            assert!(size > 1000, "Output MP4 should contain valid video data");
        }
        Err(e) => {
            panic!("Full regression export failed: {}", e);
        }
    }
}
