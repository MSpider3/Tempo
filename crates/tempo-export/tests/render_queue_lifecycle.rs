use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use tempo_export::render_queue::{ExportJob, ExportPreset, ExportQueueEvent, JobStatus, RenderQueue, RenderRange};

#[test]
fn render_queue_lifecycle() {
    let mut queue = RenderQueue::new();

    // 1. Enqueue 3 jobs with different presets
    let dest1 = PathBuf::from("/tmp/test_lifecycle_job1.mp4");
    let dest2 = PathBuf::from("/tmp/test_lifecycle_job2.mp4");
    let dest3 = PathBuf::from("/tmp/test_lifecycle_job3.mp4");

    for p in [&dest1, &dest2, &dest3] {
        if p.exists() {
            let _ = std::fs::remove_file(p);
        }
    }

    let job1 = ExportJob::new("Job 1 (YouTube)", ExportPreset::YouTube1080p, dest1.clone(), RenderRange::EntireTimeline);
    let job2 = ExportJob::new("Job 2 (TikTok)", ExportPreset::TikTokVertical, dest2.clone(), RenderRange::EntireTimeline);
    let job3 = ExportJob::new("Job 3 (Custom)", ExportPreset::Custom, dest3.clone(), RenderRange::EntireTimeline);

    let id1 = queue.add_job(job1);
    let id2 = queue.add_job(job2);
    let id3 = queue.add_job(job3);

    assert_eq!(queue.jobs().len(), 3);
    assert_eq!(queue.get_job(id1).unwrap().status, JobStatus::Queued);
    assert_eq!(queue.get_job(id2).unwrap().status, JobStatus::Queued);
    assert_eq!(queue.get_job(id3).unwrap().status, JobStatus::Queued);

    // 2. Test Pause & Resume on Queued / Pending Job
    assert!(queue.pause_job(id2));
    assert_eq!(queue.get_job(id2).unwrap().status, JobStatus::Paused);

    assert!(queue.resume_job(id2));
    assert_eq!(queue.get_job(id2).unwrap().status, JobStatus::Rendering); // resumed state

    // 3. Start batch execution with synthetic frame generation
    queue.start_batch(|job| {
        let frames = 12;
        let width = job.settings.width;
        let height = job.settings.height;
        let frame_size = (width * height * 4) as usize;
        (
            frames,
            Box::new(move |_frame_idx| {
                // Synthetic black frame
                vec![0u8; frame_size]
            }),
        )
    });

    // Collect events with timeout
    let mut completed = Vec::new();
    let mut cancelled = Vec::new();
    let mut progress_count = 0;

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Some(event) = queue.try_recv_event() {
            match event {
                ExportQueueEvent::JobProgress { id: _, progress, .. } => {
                    if progress > 0.0 {
                        progress_count += 1;
                    }
                }
                ExportQueueEvent::JobCompleted { id, .. } => {
                    completed.push(id);
                }
                ExportQueueEvent::JobCancelled { id } => {
                    cancelled.push(id);
                }
                ExportQueueEvent::QueueCompleted => {
                    break;
                }
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    // Check that at least some jobs completed
    assert!(!completed.is_empty(), "At least one job should have completed in the batch");
    assert!(progress_count > 0, "Progress events must be received");

    // Clean up files
    for p in [&dest1, &dest2, &dest3] {
        if p.exists() {
            let _ = std::fs::remove_file(p);
        }
    }
}
