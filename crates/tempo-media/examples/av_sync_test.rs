use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use tempo_audio::{AudioEngine, MAX_AV_DRIFT_US};
use tempo_media::FfmpegDecoder;
use tempo_render::{FrameCache, FrameCacheKey, WgpuRenderer};
use uuid::Uuid;

fn get_rss_bytes() -> usize {
    if let Ok(mut file) = File::open("/proc/self/statm") {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            let parts: Vec<&str> = contents.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(resident_pages) = parts[1].parse::<usize>() {
                    let page_size = 4096;
                    return resident_pages * page_size;
                }
            }
        }
    }
    0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Tempo A/V Sync & Playback Engine Test ===");

    let media_path = "tests/media/sample_10s_sync.mp4";
    if !Path::new(media_path).exists() {
        eprintln!("Error: Test media {} does not exist", media_path);
        std::process::exit(1);
    }

    let mut decoder = FfmpegDecoder::open(media_path)?;
    println!(
        "Opened media: {}x{} @ {} audio channels, {}Hz",
        decoder.video_width(),
        decoder.video_height(),
        decoder.audio_channels,
        decoder.audio_sample_rate
    );

    let audio_engine = AudioEngine::new();
    let renderer = WgpuRenderer::new().ok();
    let mut cache = FrameCache::new(256 * 1024 * 1024); // 256MB cache
    let source_id = Uuid::new_v4();

    audio_engine.play();
    println!("Playback started. Monitoring A/V sync over 10 seconds...");

    let start_time = Instant::now();
    let mut last_sample_sec = 0;
    let mut dropped_frames = 0;
    let mut max_drift_us: i64 = 0;
    let mut frames_rendered = 0;

    while start_time.elapsed() < Duration::from_millis(10_200) {
        let elapsed_sec = start_time.elapsed().as_secs();
        let current_pos_us = audio_engine.position_us();

        let key = FrameCacheKey {
            source_id,
            pts_us: current_pos_us,
        };

        let frame_pts = match cache.get(&key) {
            Some(_) => current_pos_us,
            None => {
                let decoded = decoder.decode_video_frame(current_pos_us)?;
                let pts = decoded.pts_us;
                if let Some(ref r) = renderer {
                    cache.insert(key, &decoded, &r.device, &r.queue);
                }
                pts
            }
        };

        frames_rendered += 1;
        let drift = AudioEngine::calculate_drift_us(current_pos_us, frame_pts);
        if drift.abs() > max_drift_us {
            max_drift_us = drift.abs();
        }

        // If video lags audio by > 40ms, count as dropped
        if drift > MAX_AV_DRIFT_US {
            dropped_frames += 1;
        }

        if elapsed_sec > last_sample_sec && elapsed_sec <= 10 {
            last_sample_sec = elapsed_sec;
            let rss_mb = get_rss_bytes() / (1024 * 1024);
            println!(
                "  [{:02}s] Audio Pos: {:>7}ms | Video PTS: {:>7}ms | Drift: {:>4}ms | RAM: {}MB | Cache: {}MB",
                elapsed_sec,
                current_pos_us / 1000,
                frame_pts / 1000,
                drift / 1000,
                rss_mb,
                cache.current_bytes() / (1024 * 1024)
            );
            assert!(
                drift.abs() <= MAX_AV_DRIFT_US,
                "Drift {}ms exceeded MAX_AV_DRIFT_US (40ms) at second {}",
                drift / 1000,
                elapsed_sec
            );
        }

        thread::sleep(Duration::from_millis(30)); // simulate ~33fps render loop
    }

    println!("Total frames rendered: {}", frames_rendered);
    println!("Dropped frames: {}", dropped_frames);
    println!("Max measured drift: {} µs ({} ms)", max_drift_us, max_drift_us / 1000);
    assert_eq!(dropped_frames, 0, "Dropped frames must be 0 during normal playback");
    assert!(
        max_drift_us <= MAX_AV_DRIFT_US,
        "Max drift {}ms exceeded 40ms limit",
        max_drift_us / 1000
    );

    // Test J/K/L Speed Ramping
    println!("\nTesting J/K/L Speed Ramping...");
    audio_engine.ramp_forward(); // 1x -> 2x
    assert!((audio_engine.speed() - 2.0).abs() < 0.05);
    audio_engine.ramp_forward(); // 2x -> 4x
    assert!((audio_engine.speed() - 4.0).abs() < 0.05);
    audio_engine.ramp_forward(); // 4x -> 8x
    assert!((audio_engine.speed() - 8.0).abs() < 0.05);
    println!("  ✓ Forward speed ramp (2x, 4x, 8x) confirmed");

    audio_engine.pause(); // K
    assert!(!audio_engine.is_playing());
    println!("  ✓ Pause (0x) confirmed");

    audio_engine.ramp_reverse(); // -1x
    assert!(audio_engine.is_playing());
    assert!((audio_engine.speed() - -1.0).abs() < 0.05);
    audio_engine.ramp_reverse(); // -2x
    assert!((audio_engine.speed() - -2.0).abs() < 0.05);
    audio_engine.ramp_reverse(); // -4x
    assert!((audio_engine.speed() - -4.0).abs() < 0.05);
    audio_engine.ramp_reverse(); // -8x
    assert!((audio_engine.speed() - -8.0).abs() < 0.05);
    println!("  ✓ Reverse speed ramp (-1x, -2x, -4x, -8x) confirmed");

    let final_rss_mb = get_rss_bytes() / (1024 * 1024);
    println!("\nFinal RAM Usage: {}MB", final_rss_mb);
    assert!(
        final_rss_mb < 500,
        "RAM usage ({}MB) exceeded 500MB ceiling",
        final_rss_mb
    );

    println!("\n=== ALL PHASE 1 PLAYBACK ENGINE CRITERIA PASSED ===");
    Ok(())
}
