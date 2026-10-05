use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use tempo_audio::AudioEngine;
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
                    return resident_pages * 4096;
                }
            }
        }
    }
    0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Tempo Playback Stress Test ===");

    let media_path = "tests/media/sample_10s_sync.mp4";
    if !Path::new(media_path).exists() {
        eprintln!("Error: Test media {} does not exist", media_path);
        std::process::exit(1);
    }

    let mut decoder = FfmpegDecoder::open(media_path)?;
    let audio_engine = AudioEngine::new();
    let renderer = WgpuRenderer::new().ok();
    let mut cache = FrameCache::new(128 * 1024 * 1024); // 128MB cache
    let source_id = Uuid::new_v4();

    // 1. Rapid Seek Stress Test: 50 seeks in 2 seconds
    println!("Testing rapid seeking (50 seeks in 2s)...");
    let start_seeks = Instant::now();
    for i in 0..50 {
        let seek_pos_us = (i * 187_333) % 9_500_000; // random jump within 10s
        audio_engine.seek(seek_pos_us);

        let key = FrameCacheKey {
            source_id,
            pts_us: seek_pos_us,
        };

        if cache.get(&key).is_none() {
            if let Ok(decoded) = decoder.decode_video_frame(seek_pos_us) {
                if let Some(ref r) = renderer {
                    cache.insert(key, &decoded, &r.device, &r.queue);
                }
            }
        }

        thread::sleep(Duration::from_millis(30));
    }
    println!("  ✓ 50 rapid seeks completed without deadlock or panic in {:?}", start_seeks.elapsed());

    // 2. Rapid Play/Pause Toggle Stress: 20 toggles in 1 second
    println!("\nTesting rapid play/pause toggling (20 toggles in 1s)...");
    let start_toggles = Instant::now();
    for _ in 0..20 {
        let is_playing = audio_engine.toggle_playback();
        let cur_pos = audio_engine.position_us();
        let key = FrameCacheKey {
            source_id,
            pts_us: cur_pos,
        };
        if is_playing {
            let _ = decoder.decode_video_frame(cur_pos);
        }
        let _ = cache.get(&key);
        thread::sleep(Duration::from_millis(40));
    }
    audio_engine.pause();
    println!("  ✓ 20 rapid toggles completed in {:?}", start_toggles.elapsed());

    // 3. Memory usage verification
    let rss_mb = get_rss_bytes() / (1024 * 1024);
    println!("\nFinal RAM Usage: {}MB", rss_mb);
    assert!(
        rss_mb < 500,
        "RAM usage ({}MB) exceeded 500MB ceiling",
        rss_mb
    );

    println!("\n=== PLAYBACK STRESS TEST PASSED ===");
    Ok(())
}
