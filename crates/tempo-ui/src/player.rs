//! Playback worker. All decoding happens on this thread; the GTK thread only
//! picks up finished frames.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam::channel::{unbounded, Receiver, Sender, TryRecvError};
use parking_lot::Mutex;
use tempo_media::{FfmpegDecoder, VideoFrame};
use tempo_timeline::{Clip, ClipProperties, ClipType, Project, TrackKind};
use uuid::Uuid;

/// How many container files may be open for decoding at once.
const MAX_OPEN_DECODERS: usize = 3;

/// One decoded picture plus the clip settings needed to place it.
pub struct Layer {
    pub frame: VideoFrame,
    pub props: ClipProperties,
}

/// An immutable copy of what the worker needs from the project.
pub struct Snapshot {
    /// Video tracks, bottom first. Each holds its clips sorted by time.
    tracks: Vec<Vec<Clip>>,
    sources: HashMap<Uuid, PathBuf>,
    pub duration_us: i64,
    pub frame_us: i64,
}

impl Snapshot {
    pub fn from_project(project: &Project) -> Self {
        let mut video: Vec<_> = project
            .timeline
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video && t.enabled)
            .collect();
        video.sort_by_key(|t| t.kind_index);
        let sources = project
            .sources
            .iter()
            .filter(|(_, s)| !s.is_missing)
            .map(|(id, s)| {
                let path = match (&s.proxy_path, s.proxy_ready) {
                    (Some(p), true) if p.exists() => p.clone(),
                    _ => s.path.clone(),
                };
                (*id, path)
            })
            .collect();
        Self {
            tracks: video.iter().map(|t| t.clips.clone()).collect(),
            sources,
            duration_us: project
                .timeline
                .tracks
                .iter()
                .map(|t| t.duration_us())
                .max()
                .unwrap_or(0),
            frame_us: project.fps.frame_duration_us().max(1),
        }
    }

    /// A one-clip snapshot used to preview a Media Pool item.
    pub fn from_source(id: Uuid, path: PathBuf, duration_us: i64, frame_us: i64) -> Self {
        let clip = Clip::new(Uuid::nil(), id, ClipType::Video, "", 0, duration_us, 0, duration_us);
        Self {
            tracks: vec![vec![clip]],
            sources: HashMap::from([(id, path)]),
            duration_us,
            frame_us: frame_us.max(1),
        }
    }
}

enum Cmd {
    Timeline(Arc<Snapshot>),
    Source(Option<Arc<Snapshot>>),
    Seek { us: i64, exact: bool },
    Play(f64),
    Pause,
    MaxHeight(u32),
    Quit,
}

#[derive(Default)]
struct Shared {
    position_us: AtomicI64,
    duration_us: AtomicI64,
    playing: AtomicBool,
    serial: AtomicU64,
    dropped: AtomicU32,
    layers: Mutex<Option<Vec<Layer>>>,
}

/// Handle owned by the UI. Cheap to clone.
#[derive(Clone)]
pub struct Player {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        let shared = Arc::new(Shared::default());
        let worker_shared = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("tempo-playback".into())
            .spawn(move || Worker::new(rx, worker_shared).run());
        if let Err(e) = spawned {
            tracing::error!("could not start playback thread: {e}");
        }
        Self { tx, shared }
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn set_timeline(&self, snapshot: Snapshot) {
        self.send(Cmd::Timeline(Arc::new(snapshot)));
    }

    /// Show a single source clip instead of the timeline; `None` returns to the timeline.
    pub fn set_source(&self, snapshot: Option<Snapshot>) {
        self.send(Cmd::Source(snapshot.map(Arc::new)));
    }

    /// `exact = false` shows the nearest keyframe, which is what a drag wants.
    pub fn seek(&self, us: i64, exact: bool) {
        let us = us.max(0);
        self.shared.position_us.store(us, Ordering::Release);
        self.send(Cmd::Seek { us, exact });
    }

    pub fn play(&self, speed: f64) {
        self.send(Cmd::Play(speed));
    }

    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }

    pub fn set_max_height(&self, height: u32) {
        self.send(Cmd::MaxHeight(height));
    }

    pub fn position_us(&self) -> i64 {
        self.shared.position_us.load(Ordering::Acquire)
    }

    pub fn duration_us(&self) -> i64 {
        self.shared.duration_us.load(Ordering::Acquire)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    pub fn dropped_frames(&self) -> u32 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Changes every time a new picture is ready.
    pub fn serial(&self) -> u64 {
        self.shared.serial.load(Ordering::Acquire)
    }

    pub fn take_layers(&self) -> Option<Vec<Layer>> {
        self.shared.layers.lock().take()
    }

    pub fn shutdown(&self) {
        self.send(Cmd::Quit);
    }
}

struct Worker {
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    timeline: Arc<Snapshot>,
    source: Option<Arc<Snapshot>>,
    timeline_pos: i64,
    decoders: Vec<(Uuid, FfmpegDecoder)>,
    max_height: u32,
    playing: bool,
    speed: f64,
    clock_start: Instant,
    clock_base_us: i64,
    pos_us: i64,
    exact: bool,
    dirty: bool,
    last_shown: Vec<(Uuid, i64)>,
}

impl Worker {
    fn new(rx: Receiver<Cmd>, shared: Arc<Shared>) -> Self {
        Self {
            rx,
            shared,
            timeline: Arc::new(Snapshot {
                tracks: Vec::new(),
                sources: HashMap::new(),
                duration_us: 0,
                frame_us: 33_333,
            }),
            source: None,
            timeline_pos: 0,
            decoders: Vec::new(),
            max_height: 540,
            playing: false,
            speed: 1.0,
            clock_start: Instant::now(),
            clock_base_us: 0,
            pos_us: 0,
            exact: true,
            dirty: false,
            last_shown: Vec::new(),
        }
    }

    fn active(&self) -> Arc<Snapshot> {
        self.source.clone().unwrap_or_else(|| self.timeline.clone())
    }

    fn restart_clock(&mut self) {
        self.clock_start = Instant::now();
        self.clock_base_us = self.pos_us;
    }

    fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
        self.shared.playing.store(playing, Ordering::Release);
        self.restart_clock();
    }

    /// Returns false when the worker should stop.
    fn handle(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Timeline(s) => {
                // Drop decoders for files that are no longer referenced.
                self.decoders.retain(|(id, _)| s.sources.contains_key(id) || self.source.is_some());
                self.timeline = s;
                self.dirty = true;
                self.last_shown.clear();
            }
            Cmd::Source(s) => {
                if self.source.is_none() {
                    self.timeline_pos = self.pos_us;
                }
                self.pos_us = if s.is_some() { 0 } else { self.timeline_pos };
                self.source = s;
                self.set_playing(false);
                self.exact = true;
                self.dirty = true;
                self.last_shown.clear();
            }
            Cmd::Seek { us, exact } => {
                self.pos_us = us;
                self.exact = exact;
                self.dirty = true;
                self.restart_clock();
            }
            Cmd::Play(speed) => {
                self.speed = speed;
                self.exact = speed > 0.0 && speed <= 2.0;
                self.set_playing(true);
            }
            Cmd::Pause => {
                self.set_playing(false);
                self.exact = true;
                self.dirty = true;
            }
            Cmd::MaxHeight(h) => {
                self.max_height = h;
                for (_, d) in &mut self.decoders {
                    d.set_max_output_height(h);
                }
                self.dirty = true;
                self.last_shown.clear();
            }
            Cmd::Quit => return false,
        }
        true
    }

    fn run(mut self) {
        loop {
            // Idle: block until the UI asks for something. No polling, no CPU.
            if !self.playing && !self.dirty {
                match self.rx.recv() {
                    Ok(cmd) => {
                        if !self.handle(cmd) {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
            // Take everything that is queued so a burst of seeks costs one decode.
            loop {
                match self.rx.try_recv() {
                    Ok(cmd) => {
                        if !self.handle(cmd) {
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }

            let snap = self.active();
            self.shared.duration_us.store(snap.duration_us, Ordering::Release);

            if self.playing {
                let elapsed = self.clock_start.elapsed().as_micros() as f64;
                let pos = self.clock_base_us + (elapsed * self.speed) as i64;
                // Stop at the end going forward, or at the start going backward.
                let ended = (self.speed > 0.0 && pos >= snap.duration_us) || (self.speed < 0.0 && pos <= 0);
                if ended {
                    self.pos_us = pos.clamp(0, snap.duration_us.max(0));
                    self.set_playing(false);
                    self.exact = true;
                } else {
                    self.pos_us = pos;
                }
            }
            self.shared.position_us.store(self.pos_us, Ordering::Release);

            let started = Instant::now();
            self.render(&snap);
            self.dirty = false;

            if self.playing {
                let frame = Duration::from_micros((snap.frame_us as f64 / self.speed.abs().max(1.0)) as u64);
                let spent = started.elapsed();
                if spent > frame {
                    self.shared.dropped.fetch_add((spent.as_micros() / frame.as_micros().max(1)) as u32, Ordering::Relaxed);
                } else {
                    // Sleep to the next frame, but wake early if the UI sends a command.
                    if let Ok(cmd) = self.rx.recv_timeout(frame - spent) {
                        if !self.handle(cmd) {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Decode what is under the playhead and publish it, unless it is what is already shown.
    fn render(&mut self, snap: &Snapshot) {
        let pos = self.pos_us;
        let mut wanted: Vec<(Uuid, i64, &Clip)> = Vec::new();
        for clips in &snap.tracks {
            if let Some(clip) = clips.iter().find(|c| c.contains_point(pos)) {
                if matches!(clip.clip_type, ClipType::Video | ClipType::Image) {
                    let offset = clip.source_offset_at(pos);
                    wanted.push((clip.source_id, offset - offset % snap.frame_us, clip));
                }
            }
        }

        let key: Vec<(Uuid, i64)> = wanted.iter().map(|(id, off, _)| (*id, *off)).collect();
        if key == self.last_shown && self.exact {
            return;
        }

        let mut layers = Vec::with_capacity(wanted.len());
        for (source_id, offset, clip) in wanted {
            let Some(path) = snap.sources.get(&source_id) else { continue };
            let exact = self.exact;
            let Some(decoder) = self.decoder_for(source_id, path) else { continue };
            let result = if exact { decoder.decode_video_frame(offset) } else { decoder.decode_keyframe(offset) };
            match result {
                Ok(frame) => layers.push(Layer { frame, props: clip.properties.clone() }),
                Err(e) => tracing::debug!("decode failed for {}: {e}", path.display()),
            }
        }

        self.last_shown = if self.exact { key } else { Vec::new() };
        *self.shared.layers.lock() = Some(layers);
        self.shared.serial.fetch_add(1, Ordering::AcqRel);
    }

    fn decoder_for(&mut self, id: Uuid, path: &PathBuf) -> Option<&mut FfmpegDecoder> {
        if let Some(idx) = self.decoders.iter().position(|(d, _)| *d == id) {
            // Move to the back: most recently used.
            let entry = self.decoders.remove(idx);
            self.decoders.push(entry);
        } else {
            match FfmpegDecoder::open(path) {
                Ok(mut d) => {
                    d.set_max_output_height(self.max_height);
                    if self.decoders.len() >= MAX_OPEN_DECODERS {
                        self.decoders.remove(0);
                    }
                    self.decoders.push((id, d));
                }
                Err(e) => {
                    tracing::warn!("cannot open {}: {e}", path.display());
                    return None;
                }
            }
        }
        self.decoders.last_mut().map(|(_, d)| d)
    }
}
