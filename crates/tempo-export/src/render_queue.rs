use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::h264::ExportSettings;


static HW_ENCODER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Check whether the VAAPI/hardware encoder is currently active.
pub fn is_hw_encoder_active() -> bool {
    HW_ENCODER_ACTIVE.load(Ordering::Relaxed)
}

/// Set whether the VAAPI/hardware encoder is currently active.
pub fn set_hw_encoder_active(active: bool) {
    HW_ENCODER_ACTIVE.store(active, Ordering::Relaxed);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportPreset {
    YouTube720p,
    YouTube1080p,
    YouTube4K,
    TikTokVertical,
    ProResMaster,
    Custom,
}

impl ExportPreset {
    pub fn name(&self) -> &'static str {
        match self {
            ExportPreset::YouTube720p => "YouTube 720p",
            ExportPreset::YouTube1080p => "YouTube 1080p",
            ExportPreset::YouTube4K => "YouTube 4K UHD",
            ExportPreset::TikTokVertical => "TikTok / Shorts (9:16)",
            ExportPreset::ProResMaster => "ProRes Master",
            ExportPreset::Custom => "Custom",
        }
    }

    pub fn to_settings(&self, destination: PathBuf) -> ExportSettings {
        match self {
            ExportPreset::YouTube720p => ExportSettings {
                width: 1280,
                height: 720,
                fps: 60.0,
                crf: 18,
                output_path: destination,
                audio_source_path: None,
                audio_start_sec: None,
            },
            ExportPreset::YouTube1080p => ExportSettings {
                width: 1920,
                height: 1080,
                fps: 30.0,
                crf: 18,
                output_path: destination,
                audio_source_path: None,
                audio_start_sec: None,
            },
            ExportPreset::YouTube4K => ExportSettings {
                width: 3840,
                height: 2160,
                fps: 30.0,
                crf: 18,
                output_path: destination,
                audio_source_path: None,
                audio_start_sec: None,
            },
            ExportPreset::TikTokVertical => ExportSettings {
                width: 1080,
                height: 1920,
                fps: 30.0,
                crf: 20,
                output_path: destination,
                audio_source_path: None,
                audio_start_sec: None,
            },
            ExportPreset::ProResMaster => ExportSettings {
                width: 1920,
                height: 1080,
                fps: 24.0,
                crf: 12,
                output_path: destination,
                audio_source_path: None,
                audio_start_sec: None,
            },
            ExportPreset::Custom => ExportSettings {
                width: 1920,
                height: 1080,
                fps: 30.0,
                crf: 18,
                output_path: destination,
                audio_source_path: None,
                audio_start_sec: None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderRange {
    EntireTimeline,
    InOutRange { in_us: i64, out_us: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Rendering,
    Paused,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct ExportJob {
    pub id: Uuid,
    pub name: String,
    pub preset: ExportPreset,
    pub destination: PathBuf,
    pub settings: ExportSettings,
    pub range: RenderRange,
    pub status: JobStatus,
    pub progress: f32, // 0.0 to 1.0
    pub current_frame: usize,
    pub total_frames: usize,
    pub fps: f32,
    pub eta_seconds: u64,
    pub cancel_token: Arc<AtomicBool>,
    pub pause_token: Arc<AtomicBool>,
}

impl ExportJob {
    pub fn new(
        name: impl Into<String>,
        preset: ExportPreset,
        destination: PathBuf,
        range: RenderRange,
    ) -> Self {
        let settings = preset.to_settings(destination.clone());
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            preset,
            destination,
            settings,
            range,
            status: JobStatus::Queued,
            progress: 0.0,
            current_frame: 0,
            total_frames: 0,
            fps: 0.0,
            eta_seconds: 0,
            cancel_token: Arc::new(AtomicBool::new(false)),
            pause_token: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancel_token.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        self.pause_token.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.pause_token.store(false, Ordering::SeqCst);
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, JobStatus::Rendering | JobStatus::Paused)
    }
}

#[derive(Debug, Clone)]
pub enum ExportQueueEvent {
    JobAdded(Uuid),
    JobStarted(Uuid),
    JobProgress {
        id: Uuid,
        progress: f32,
        current_frame: usize,
        total_frames: usize,
        fps: f32,
        eta_seconds: u64,
    },
    JobStatusChanged {
        id: Uuid,
        status: JobStatus,
    },
    JobCompleted {
        id: Uuid,
        destination: PathBuf,
    },
    JobFailed {
        id: Uuid,
        error: String,
    },
    JobCancelled {
        id: Uuid,
    },
    QueueCompleted,
}

pub struct RenderQueue {
    jobs: Arc<Mutex<Vec<ExportJob>>>,
    event_tx: Sender<ExportQueueEvent>,
    event_rx: Receiver<ExportQueueEvent>,
    worker_handle: Option<JoinHandle<()>>,
}

impl RenderQueue {
    pub fn new() -> Self {
        let (event_tx, event_rx) = channel();
        Self {
            jobs: Arc::new(Mutex::new(Vec::new())),
            event_tx,
            event_rx,
            worker_handle: None,
        }
    }

    pub fn event_sender(&self) -> Sender<ExportQueueEvent> {
        self.event_tx.clone()
    }

    pub fn try_recv_event(&self) -> Option<ExportQueueEvent> {
        self.event_rx.try_recv().ok()
    }

    pub fn add_job(&mut self, job: ExportJob) -> Uuid {
        let id = job.id;
        {
            let mut jobs = self.jobs.lock().unwrap();
            jobs.push(job);
        }
        let _ = self.event_tx.send(ExportQueueEvent::JobAdded(id));
        id
    }

    pub fn jobs(&self) -> Vec<ExportJob> {
        self.jobs.lock().unwrap().clone()
    }

    pub fn get_job(&self, id: Uuid) -> Option<ExportJob> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.id == id)
            .cloned()
    }

    pub fn cancel_job(&mut self, id: Uuid) -> bool {
        let jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.iter().find(|j| j.id == id) {
            job.cancel();
            true
        } else {
            false
        }
    }

    pub fn pause_job(&mut self, id: Uuid) -> bool {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.pause();
            job.status = JobStatus::Paused;
            let _ = self.event_tx.send(ExportQueueEvent::JobStatusChanged {
                id,
                status: JobStatus::Paused,
            });
            true
        } else {
            false
        }
    }

    pub fn resume_job(&mut self, id: Uuid) -> bool {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.resume();
            if job.status == JobStatus::Paused {
                job.status = JobStatus::Rendering;
                let _ = self.event_tx.send(ExportQueueEvent::JobStatusChanged {
                    id,
                    status: JobStatus::Rendering,
                });
            }
            true
        } else {
            false
        }
    }

    pub fn clear_completed(&mut self) {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.retain(|j| !matches!(j.status, JobStatus::Completed | JobStatus::Cancelled));
    }

    pub fn has_active_jobs(&self) -> bool {
        self.jobs.lock().unwrap().iter().any(|j| j.is_active())
    }

    /// Spawns a background thread to process all queued jobs.
    pub fn start_batch<F>(&mut self, mut frame_generator_factory: F)
    where
        F: FnMut(&ExportJob) -> (usize, Box<dyn FnMut(usize) -> Vec<u8> + Send + 'static>)
            + Send
            + 'static,
    {
        let jobs_ref = Arc::clone(&self.jobs);
        let event_tx = self.event_tx.clone();

        let handle = thread::spawn(move || {
            loop {
                // Find next queued job
                let (job_id, destination, cancel_token, pause_token, settings) = {
                    let mut jobs = jobs_ref.lock().unwrap();
                    let next_job = jobs.iter_mut().find(|j| j.status == JobStatus::Queued);
                    if let Some(job) = next_job {
                        job.status = JobStatus::Rendering;
                        (
                            job.id,
                            job.destination.clone(),
                            Arc::clone(&job.cancel_token),
                            Arc::clone(&job.pause_token),
                            job.settings.clone(),
                        )
                    } else {
                        break;
                    }
                };

                let _ = event_tx.send(ExportQueueEvent::JobStarted(job_id));

                // Get frame generator for this job
                let dummy_job = ExportJob {
                    id: job_id,
                    name: String::new(),
                    preset: ExportPreset::Custom,
                    destination: destination.clone(),
                    settings: settings.clone(),
                    range: RenderRange::EntireTimeline,
                    status: JobStatus::Rendering,
                    progress: 0.0,
                    current_frame: 0,
                    total_frames: 0,
                    fps: 0.0,
                    eta_seconds: 0,
                    cancel_token: Arc::clone(&cancel_token),
                    pause_token: Arc::clone(&pause_token),
                };

                let (total_frames, mut frame_generator) = frame_generator_factory(&dummy_job);

                let start_time = Instant::now();
                let width = settings.width.max(16);
                let height = settings.height.max(16);
                let fps = if settings.fps <= 0.0 { 24.0 } else { settings.fps };
                let crf = settings.crf.min(51);

                let output_str = match destination.to_str() {
                    Some(s) => s,
                    None => {
                        let mut jobs = jobs_ref.lock().unwrap();
                        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                            job.status = JobStatus::Failed("Invalid destination path".into());
                        }
                        let _ = event_tx.send(ExportQueueEvent::JobFailed {
                            id: job_id,
                            error: "Invalid destination path".into(),
                        });
                        continue;
                    }
                };

                let child_res = Command::new("ffmpeg")
                    .args([
                        "-y",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        "rgba",
                        "-video_size",
                        &format!("{}x{}", width, height),
                        "-framerate",
                        &format!("{}", fps),
                        "-i",
                        "pipe:0",
                        "-c:v",
                        "libx264",
                        "-preset",
                        "veryfast",
                        "-crf",
                        &format!("{}", crf),
                        "-pix_fmt",
                        "yuv420p",
                        output_str,
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn();

                let mut child = match child_res {
                    Ok(c) => c,
                    Err(e) => {
                        let err_msg = format!("Failed to spawn ffmpeg: {}", e);
                        let mut jobs = jobs_ref.lock().unwrap();
                        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                            job.status = JobStatus::Failed(err_msg.clone());
                        }
                        let _ = event_tx.send(ExportQueueEvent::JobFailed {
                            id: job_id,
                            error: err_msg,
                        });
                        continue;
                    }
                };

                let mut cancelled = false;
                let mut encode_err = None;

                {
                    if let Some(mut stdin) = child.stdin.take() {
                        for frame_idx in 0..total_frames {
                            // Check cancellation before every frame (<= 42ms response)
                            if cancel_token.load(Ordering::SeqCst) {
                                cancelled = true;
                                break;
                            }

                            // Check pause state
                            while pause_token.load(Ordering::SeqCst) {
                                if cancel_token.load(Ordering::SeqCst) {
                                    cancelled = true;
                                    break;
                                }
                                thread::sleep(Duration::from_millis(15));
                            }
                            if cancelled {
                                break;
                            }

                            let frame_bytes = frame_generator(frame_idx);
                            let chunk_size = 64 * 1024;
                            let mut offset = 0;
                            while offset < frame_bytes.len() {
                                if cancel_token.load(Ordering::SeqCst) {
                                    cancelled = true;
                                    break;
                                }
                                let end = (offset + chunk_size).min(frame_bytes.len());
                                if let Err(e) = stdin.write_all(&frame_bytes[offset..end]) {
                                    encode_err = Some(format!("Write frame error: {}", e));
                                    break;
                                }
                                offset = end;
                            }
                            if cancelled {
                                break;
                            }


                            let elapsed = start_time.elapsed().as_secs_f32();
                            let current = frame_idx + 1;
                            let progress = current as f32 / total_frames.max(1) as f32;
                            let current_fps = if elapsed > 0.0 {
                                current as f32 / elapsed
                            } else {
                                0.0
                            };
                            let remaining = total_frames.saturating_sub(current);
                            let eta_secs = if current_fps > 0.0 {
                                (remaining as f32 / current_fps) as u64
                            } else {
                                0
                            };

                            {
                                let mut jobs = jobs_ref.lock().unwrap();
                                if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                                    job.progress = progress;
                                    job.current_frame = current;
                                    job.total_frames = total_frames;
                                    job.fps = current_fps;
                                    job.eta_seconds = eta_secs;
                                }
                            }

                            if frame_idx % 4 == 0 || current == total_frames {
                                let _ = event_tx.send(ExportQueueEvent::JobProgress {
                                    id: job_id,
                                    progress,
                                    current_frame: current,
                                    total_frames,
                                    fps: current_fps,
                                    eta_seconds: eta_secs,
                                });
                            }
                        }
                    }
                }

                if cancelled {
                    let _ = child.kill();
                    let _ = child.wait();
                    if destination.exists() {
                        let _ = std::fs::remove_file(&destination);
                    }
                    let mut jobs = jobs_ref.lock().unwrap();
                    if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                        job.status = JobStatus::Cancelled;
                    }
                    let _ = event_tx.send(ExportQueueEvent::JobCancelled { id: job_id });
                    continue;
                }

                if let Some(err) = encode_err {
                    let _ = child.kill();
                    let _ = child.wait();
                    if destination.exists() {
                        let _ = std::fs::remove_file(&destination);
                    }
                    let mut jobs = jobs_ref.lock().unwrap();
                    if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                        job.status = JobStatus::Failed(err.clone());
                    }
                    let _ = event_tx.send(ExportQueueEvent::JobFailed {
                        id: job_id,
                        error: err,
                    });
                    continue;
                }

                let output_res = child.wait_with_output();
                match output_res {
                    Ok(out) if out.status.success() => {
                        let mut jobs = jobs_ref.lock().unwrap();
                        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                            job.status = JobStatus::Completed;
                            job.progress = 1.0;
                        }
                        let _ = event_tx.send(ExportQueueEvent::JobCompleted {
                            id: job_id,
                            destination: destination.clone(),
                        });
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                        if destination.exists() {
                            let _ = std::fs::remove_file(&destination);
                        }
                        let mut jobs = jobs_ref.lock().unwrap();
                        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                            job.status = JobStatus::Failed(stderr.clone());
                        }
                        let _ = event_tx.send(ExportQueueEvent::JobFailed {
                            id: job_id,
                            error: stderr,
                        });
                    }
                    Err(e) => {
                        let err = format!("Child wait error: {}", e);
                        if destination.exists() {
                            let _ = std::fs::remove_file(&destination);
                        }
                        let mut jobs = jobs_ref.lock().unwrap();
                        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                            job.status = JobStatus::Failed(err.clone());
                        }
                        let _ = event_tx.send(ExportQueueEvent::JobFailed {
                            id: job_id,
                            error: err,
                        });
                    }
                }
            }

            let _ = event_tx.send(ExportQueueEvent::QueueCompleted);
        });

        self.worker_handle = Some(handle);
    }
}

impl Default for RenderQueue {
    fn default() -> Self {
        Self::new()
    }
}
