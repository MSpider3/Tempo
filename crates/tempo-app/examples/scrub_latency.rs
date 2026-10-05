use std::env;
use std::path::Path;
use std::time::Instant;
use tempo_media::FfmpegDecoder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let media_path = if args.len() > 1 && !args[1].starts_with("--") {
        args[1].clone()
    } else {
        "tests/media/sample_1080p_h264.mp4".to_string()
    };

    let mut num_seeks = 100;
    let mut max_latency_ms = 100.0;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--seeks" && i + 1 < args.len() {
            num_seeks = args[i + 1].parse().unwrap_or(100);
            i += 1;
        } else if args[i] == "--max-latency-ms" && i + 1 < args.len() {
            max_latency_ms = args[i + 1].parse().unwrap_or(100.0);
            i += 1;
        }
        i += 1;
    }

    println!("=== Tempo Scrub Latency Benchmark ===");
    println!("Media: {}", media_path);
    println!("Seeks: {}", num_seeks);
    println!("Threshold p95: <= {} ms", max_latency_ms);

    if !Path::new(&media_path).exists() {
        eprintln!("Error: media file {} not found", media_path);
        std::process::exit(1);
    }

    let mut decoder = FfmpegDecoder::open(&media_path)?;
    let duration_us = 4_500_000i64; // ~4.5s of sample_1080p_h264.mp4

    let mut latencies_ms = Vec::with_capacity(num_seeks);

    // Warm up
    let _ = decoder.decode_video_frame(100_000);

    for s in 0..num_seeks {
        // Pseudo-random deterministic seek target across duration
        let target_us = ((s * 7919) as i64 * 33_333) % duration_us;
        let t0 = Instant::now();
        let _ = decoder.decode_video_frame(target_us)?;
        let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
        latencies_ms.push(elapsed);
    }

    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let min = latencies_ms.first().copied().unwrap_or(0.0);
    let max = latencies_ms.last().copied().unwrap_or(0.0);
    let avg = latencies_ms.iter().sum::<f64>() / latencies_ms.len() as f64;
    let p95_idx = ((num_seeks as f64 * 0.95) as usize).min(num_seeks - 1);
    let p95 = latencies_ms[p95_idx];

    println!("\nLatency Statistics (ms):");
    println!("  Min: {:.2} ms", min);
    println!("  Avg: {:.2} ms", avg);
    println!("  p95: {:.2} ms", p95);
    println!("  Max: {:.2} ms", max);

    assert!(
        p95 <= max_latency_ms,
        "p95 latency {:.2}ms exceeded threshold {:.2}ms",
        p95,
        max_latency_ms
    );

    println!("\n✓ Scrub latency benchmark PASSED (p95 < {}ms)", max_latency_ms);
    Ok(())
}
