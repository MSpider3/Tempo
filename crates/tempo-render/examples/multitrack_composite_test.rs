use std::env;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Instant;
use tempo_media::FfmpegDecoder;
use tempo_render::{LayerDesc, WgpuRenderer};
use wgpu::{
    Extent3d, ImageCopyTexture, ImageDataLayout, Origin3d, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
};

fn get_process_rss_mb() -> f64 {
    if let Ok(mut file) = File::open("/proc/self/statm") {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            let parts: Vec<&str> = contents.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(resident_pages) = parts[1].parse::<u64>() {
                    let page_size_kb = 4; // standard 4KB pages on Linux x86_64
                    return (resident_pages * page_size_kb) as f64 / 1024.0;
                }
            }
        }
    }
    0.0
}

fn main() {
    println!("=== Multi-Track Compositing Test ===");
    let args: Vec<String> = env::args().collect();

    let mut media_path = "tests/media/sample_1080p_h264.mp4".to_string();
    let mut num_tracks: usize = 4;
    let mut num_frames: usize = 100;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--tracks" => {
                if i + 1 < args.len() {
                    num_tracks = args[i + 1].parse().unwrap_or(4);
                    i += 1;
                }
            }
            "--frames" => {
                if i + 1 < args.len() {
                    num_frames = args[i + 1].parse().unwrap_or(100);
                    i += 1;
                }
            }
            arg if !arg.starts_with("--") => {
                media_path = arg.to_string();
            }
            _ => {}
        }
        i += 1;
    }

    println!(
        "Config: media='{}', tracks={}, frames={}",
        media_path, num_tracks, num_frames
    );

    // Initialize wgpu renderer
    let renderer = WgpuRenderer::new().expect("Failed to initialize WgpuRenderer");

    // Load sample video frames
    let mut decoder = FfmpegDecoder::open(Path::new(&media_path)).expect("Failed to open media file");
    let width = decoder.video_width();
    let height = decoder.video_height();
    println!("Video stream: {}x{}", width, height);

    // Decode sample frames to upload as source textures
    let mut decoded_frames = Vec::new();
    let mut current_pts_us = 0i64;
    while decoded_frames.len() < num_tracks {
        match decoder.decode_video_frame(current_pts_us) {
            Ok(frame) => {
                decoded_frames.push(frame);
                current_pts_us += 33_333;
            }
            Err(_) => {
                if decoded_frames.is_empty() {
                    panic!("Failed to decode any frames from {}", media_path);
                }
                break;
            }
        }
    }

    // If video is short, duplicate decoded frames
    while decoded_frames.len() < num_tracks {
        let clone_frame = decoded_frames[0].clone();
        decoded_frames.push(clone_frame);
    }

    // Create GPU textures for each track
    let mut textures = Vec::new();
    let mut views = Vec::new();

    for (t_idx, frame) in decoded_frames.iter().take(num_tracks).enumerate() {
        let rgba_data = &frame.data;
        let texture = renderer.device.create_texture(&TextureDescriptor {
            label: Some(&format!("Track {} Source Texture", t_idx + 1)),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });

        renderer.queue.write_texture(
            ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            rgba_data,
            ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&TextureViewDescriptor::default());
        textures.push(texture);
        views.push(view);
    }

    // Setup 4 track layer layout with distinct transforms:
    // V1: Background (fullscreen)
    // V2: Top-right PiP (scale 0.45, offset [0.5, 0.5])
    // V3: Bottom-left PiP (scale 0.45, offset [-0.5, -0.5], opacity 0.85)
    // V4: Center PiP overlay with slight rotation
    let mut layers = Vec::new();
    for (idx, view) in views.iter().enumerate() {
        let layer = match idx {
            0 => LayerDesc::new(view),
            1 => LayerDesc::new(view).with_scale(0.45, 0.45).with_offset(0.5, 0.5),
            2 => LayerDesc::new(view)
                .with_scale(0.45, 0.45)
                .with_offset(-0.5, -0.5)
                .with_opacity(0.85),
            _ => LayerDesc::new(view)
                .with_scale(0.35, 0.35)
                .with_offset(0.0, 0.0)
                .with_rotation(15.0)
                .with_opacity(0.9),
        };
        layers.push(layer);
    }

    println!("Starting composition benchmark for {} frames across {} tracks...", num_frames, num_tracks);

    let start_all = Instant::now();
    let mut frame_times_us = Vec::with_capacity(num_frames);
    let mut dropped_frames = 0;

    for frame_idx in 0..num_frames {
        let frame_start = Instant::now();

        // Render composite frame
        let result = renderer.composite_layers(width, height, &layers);
        let elapsed = frame_start.elapsed();
        frame_times_us.push(elapsed.as_micros() as f64);

        match result {
            Ok(rgba_output) => {
                if rgba_output.len() != (width * height * 4) as usize {
                    dropped_frames += 1;
                }
            }
            Err(e) => {
                eprintln!("Error rendering frame {}: {:?}", frame_idx, e);
                dropped_frames += 1;
            }
        }
    }

    let total_elapsed = start_all.elapsed();
    let avg_time_ms = (frame_times_us.iter().sum::<f64>() / num_frames as f64) / 1000.0;
    let min_time_ms = frame_times_us.iter().cloned().fold(f64::INFINITY, f64::min) / 1000.0;
    let max_time_ms = frame_times_us.iter().cloned().fold(0.0f64, f64::max) / 1000.0;
    let rss_mb = get_process_rss_mb();

    println!("\n=== Multi-Track Compositing Results ===");
    println!("Total Frames Rendered: {}", num_frames);
    println!("Dropped Frames:        {}", dropped_frames);
    println!("Total Time Elapsed:    {:.2} ms ({:.2}s)", total_elapsed.as_millis(), total_elapsed.as_secs_f64());
    println!("Average Frame Time:    {:.2} ms (Budget: <16.67 ms for 60fps)", avg_time_ms);
    println!("Min Frame Time:        {:.2} ms", min_time_ms);
    println!("Max Frame Time:        {:.2} ms", max_time_ms);
    println!("Process RSS Memory:    {:.2} MB (Ceiling: <2048 MB)", rss_mb);

    // Validation asserts
    assert_eq!(dropped_frames, 0, "No dropped frames permitted!");
    assert!(
        rss_mb < 2048.0,
        "Memory ceiling violated: {:.2} MB >= 2048 MB",
        rss_mb
    );

    println!("\nMulti-Track Compositing Test PASSED!");
}
