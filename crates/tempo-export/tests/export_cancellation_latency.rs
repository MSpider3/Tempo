use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};
use tempo_export::render_queue::{ExportJob, ExportPreset, ExportQueueEvent, RenderQueue, RenderRange};

#[test]
fn export_cancellation_latency() {
    let mut queue = RenderQueue::new();
    let dest = PathBuf::from("/tmp/test_cancellation_job.mp4");
    if dest.exists() {
        let _ = std::fs::remove_file(&dest);
    }

    let job = ExportJob::new(
        "Cancellation Stress Job",
        ExportPreset::YouTube1080p,
        dest.clone(),
        RenderRange::EntireTimeline,
    );
    let cancel_token = job.cancel_token.clone();
    let id = queue.add_job(job);

    // Generate 300 frames
    queue.start_batch(|job| {
        let width = job.settings.width;
        let height = job.settings.height;
        let frame_size = (width * height * 4) as usize;
        (
            300,
            Box::new(move |_frame_idx| {
                thread::sleep(Duration::from_millis(1));
                vec![0u8; frame_size]
            }),
        )
    });

    // Wait until job is actively encoding frames
    let start_wait = Instant::now();
    while start_wait.elapsed() < Duration::from_secs(5) {
        if let Some(ExportQueueEvent::JobProgress { id: p_id, current_frame, .. }) = queue.try_recv_event() {
            if p_id == id && current_frame >= 2 {
                break;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }


    // Measure cancellation response latency
    let cancel_time = Instant::now();
    cancel_token.store(true, Ordering::SeqCst);

    let mut cancelled = false;
    let mut latency = Duration::ZERO;
    while cancel_time.elapsed() < Duration::from_millis(500) {
        if let Some(ExportQueueEvent::JobCancelled { id: cancelled_id }) = queue.try_recv_event() {
            if cancelled_id == id {
                latency = cancel_time.elapsed();
                cancelled = true;
                break;
            }
        }
        thread::sleep(Duration::from_millis(1));
    }

    assert!(cancelled, "Job must emit JobCancelled event");
    println!("Measured cancellation latency: {:?}", latency);
    assert!(
        latency <= Duration::from_millis(42),
        "Cancellation latency must be <= 42ms (per Codex/Hermes spec), was {:?}",
        latency
    );

    // Scratch file must be cleaned up immediately
    assert!(!dest.exists(), "Partial output file must be cleanly deleted on cancellation");
}
