//! Playback worker. All decoding happens on this thread; the GTK thread only
//! picks up finished frames.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam::channel::{unbounded, Receiver, Sender, TryRecvError};
use parking_lot::Mutex;
use tempo_audio::AudioOutput;
use tempo_media::{AudioReader, FfmpegDecoder, VideoFrame, OUT_CHANNELS, OUT_RATE};
use tempo_timeline::{Clip, ClipProperties, ClipType, Project, TitleData, TrackKind};
use uuid::Uuid;

/// How many container files may be open for decoding at once.
const MAX_OPEN_DECODERS: usize = 3;

/// One decoded picture plus the clip settings needed to place it.
pub struct Layer {
    /// The decoded picture, for video and image clips.
    pub frame: Option<VideoFrame>,
    /// The text to draw, for title clips.
    pub title: Option<TitleData>,
    pub props: ClipProperties,
}

/// An immutable copy of what the worker needs from the project.
pub struct Snapshot {
    /// Video tracks, bottom first. Each holds its clips sorted by time.
    tracks: Vec<Vec<Clip>>,
    /// Audio tracks that are not muted, with their track volume.
    audio: Vec<(Vec<Clip>, f32)>,
    /// What the picture is decoded from: the proxy when there is one.
    sources: HashMap<Uuid, PathBuf>,
    /// What the sound is decoded from: always the original file.
    audio_sources: HashMap<Uuid, PathBuf>,
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
        let audio = project
            .timeline
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio && t.enabled)
            .flat_map(|t| {
                // For sound, the outgoing clip fades out while the incoming one fades in.
                let leads = t.dissolve_leads();
                let mut clips = t.clips.clone();
                for lead in &leads {
                    if let Some(out) = clips.iter_mut().find(|c| c.timeline_out == lead.timeline_out) {
                        out.properties.fade_out_us = out.properties.fade_out_us.max(lead.duration_us());
                    }
                }
                let mut strips = vec![(clips, t.volume)];
                if !leads.is_empty() {
                    strips.push((leads, t.volume));
                }
                strips
            })
            .collect();
        let audio_sources = project.sources.iter().filter(|(_, s)| !s.is_missing).map(|(id, s)| (*id, s.path.clone())).collect();
        // A cross dissolve is drawn as one more layer right above its track,
        // holding the lead-ins of the clips that dissolve in.
        let mut tracks = Vec::new();
        for t in &video {
            tracks.push(t.clips.clone());
            let leads = t.dissolve_leads();
            if !leads.is_empty() {
                tracks.push(leads);
            }
        }
        Self {
            tracks,
            audio,
            audio_sources,
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

    pub fn empty() -> Self {
        Self { tracks: Vec::new(), audio: Vec::new(), sources: HashMap::new(), audio_sources: HashMap::new(), duration_us: 0, frame_us: 33_333 }
    }

    /// A one-clip snapshot used to preview a Media Pool item.
    pub fn from_source(id: Uuid, path: PathBuf, duration_us: i64, frame_us: i64, has_video: bool, has_audio: bool) -> Self {
        let clip = |kind| Clip::new(Uuid::nil(), id, kind, "", 0, duration_us, 0, duration_us);
        Self {
            tracks: if has_video { vec![vec![clip(ClipType::Video)]] } else { Vec::new() },
            audio: if has_audio { vec![(vec![clip(ClipType::Audio)], 1.0)] } else { Vec::new() },
            audio_sources: HashMap::from([(id, path.clone())]),
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
    /// Output level of the last audio block, left and right, as `f32` bits.
    level: [AtomicU32; 2],
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

    /// Current output level (0.0–1.0), left and right. Zero when nothing plays.
    pub fn levels(&self) -> (f32, f32) {
        (
            f32::from_bits(self.shared.level[0].load(Ordering::Relaxed)),
            f32::from_bits(self.shared.level[1].load(Ordering::Relaxed)),
        )
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
    /// Sound output and the thread that feeds it. `None` without a sound server.
    audio: Option<Arc<AudioOutput>>,
    feeder: Sender<Feed>,
    /// True while the sound clock drives the playhead.
    audio_clock: bool,
    /// Last frame count seen and when, to notice a stalled sound device.
    audio_seen: (u64, Instant),
}

impl Worker {
    fn new(rx: Receiver<Cmd>, shared: Arc<Shared>) -> Self {
        let audio = AudioOutput::start().map(Arc::new);
        if audio.is_none() {
            tracing::warn!("no sound server found: playback will be silent");
        }
        let (feeder, feed_rx) = unbounded();
        if let Some(out) = audio.clone() {
            let spawned = std::thread::Builder::new().name("tempo-audio-feed".into()).spawn(move || feed_audio(feed_rx, out));
            if let Err(e) = spawned {
                tracing::error!("could not start the audio thread: {e}");
            }
        }
        Self {
            audio,
            feeder,
            audio_clock: false,
            audio_seen: (0, Instant::now()),
            rx,
            shared,
            timeline: Arc::new(Snapshot {
                tracks: Vec::new(),
                audio: Vec::new(),
                sources: HashMap::new(),
                audio_sources: HashMap::new(),
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

    /// Start or stop sound to match the transport. Sound plays only at normal
    /// speed; shuttling and reverse are silent and run on the wall clock.
    fn sync_audio(&mut self) {
        let Some(out) = self.audio.clone() else { return };
        let _ = self.feeder.send(Feed::Stop);
        out.clear();
        self.audio_clock = false;
        for level in &self.shared.level {
            level.store(0, Ordering::Relaxed);
        }
        if self.playing && self.speed == 1.0 {
            // Wait for the audio thread to empty its queue, so the frame count starts at zero.
            let deadline = Instant::now() + Duration::from_millis(150);
            while out.clear_pending() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            if !out.clear_pending() {
                let _ = self.feeder.send(Feed::Start { snapshot: self.active(), pos_us: self.pos_us });
                self.audio_clock = true;
                self.audio_seen = (0, Instant::now());
            }
        }
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
                // An edit while playing: the sound must follow the new timeline.
                if self.playing {
                    self.sync_audio();
                }
            }
            Cmd::Source(s) => {
                if self.source.is_none() {
                    self.timeline_pos = self.pos_us;
                }
                self.pos_us = if s.is_some() { 0 } else { self.timeline_pos };
                self.source = s;
                self.set_playing(false);
                self.sync_audio();
                self.exact = true;
                self.dirty = true;
                self.last_shown.clear();
            }
            Cmd::Seek { us, exact } => {
                self.pos_us = us;
                self.exact = exact;
                self.dirty = true;
                self.sync_audio();
            }
            Cmd::Play(speed) => {
                self.speed = speed;
                self.exact = speed > 0.0 && speed <= 2.0;
                self.set_playing(true);
                self.sync_audio();
            }
            Cmd::Pause => {
                self.set_playing(false);
                self.sync_audio();
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
            Cmd::Quit => {
                let _ = self.feeder.send(Feed::Quit);
                return false;
            }
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
                let pos = match (self.audio.clone(), self.audio_clock) {
                    (Some(out), true) => {
                        let frames = out.frames_played();
                        if frames != self.audio_seen.0 {
                            self.audio_seen = (frames, Instant::now());
                        } else if self.audio_seen.1.elapsed() > Duration::from_millis(700) {
                            // The sound device has stopped asking for data; carry on by the wall clock.
                            tracing::warn!("sound output stalled; continuing without the audio clock");
                            self.pos_us = self.clock_base_us + (frames * 1_000_000 / OUT_RATE as u64) as i64;
                            self.audio_clock = false;
                            self.restart_clock();
                        }
                        let (l, r) = out.peaks();
                        self.shared.level[0].store(l.to_bits(), Ordering::Relaxed);
                        self.shared.level[1].store(r.to_bits(), Ordering::Relaxed);
                        self.clock_base_us + (frames * 1_000_000 / OUT_RATE as u64) as i64
                    }
                    _ => {
                        let elapsed = self.clock_start.elapsed().as_micros() as f64;
                        self.clock_base_us + (elapsed * self.speed) as i64
                    }
                };
                // Stop at the end going forward, or at the start going backward.
                let ended = (self.speed > 0.0 && pos >= snap.duration_us) || (self.speed < 0.0 && pos <= 0);
                if ended {
                    self.pos_us = pos.clamp(0, snap.duration_us.max(0));
                    self.set_playing(false);
                    self.sync_audio();
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
        // What is under the playhead on each track, bottom first. Title clips need
        // no decoding; the viewer draws their text.
        let wanted: Vec<(&Clip, i64)> = snap
            .tracks
            .iter()
            .filter_map(|clips| clips.iter().find(|c| c.contains_point(pos)))
            .filter(|c| c.properties.enabled)
            .map(|c| {
                let offset = c.source_offset_at(pos);
                (c, offset - offset % snap.frame_us)
            })
            .collect();

        // Skip the work if this exact picture is already on screen. A title's key
        // is its clip and how far through a fade it is.
        let key: Vec<(Uuid, i64)> = wanted
            .iter()
            .map(|(c, offset)| match c.clip_type {
                ClipType::Title => (c.id, (c.fade_factor(pos) * 1000.0) as i64),
                _ => (c.source_id, *offset),
            })
            .collect();
        if key == self.last_shown && self.exact {
            return;
        }

        let mut layers = Vec::with_capacity(wanted.len());
        for (clip, offset) in wanted {
            let mut props = clip.properties.clone();
            props.opacity *= clip.fade_factor(pos);
            if clip.clip_type == ClipType::Title {
                layers.push(Layer { frame: None, title: clip.title_data.clone(), props });
                continue;
            }
            let Some(path) = snap.sources.get(&clip.source_id) else { continue };
            let exact = self.exact;
            let Some(decoder) = self.decoder_for(clip.source_id, path) else { continue };
            let result = if exact { decoder.decode_video_frame(offset) } else { decoder.decode_keyframe(offset) };
            match result {
                Ok(frame) => layers.push(Layer { frame: Some(frame), title: None, props }),
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

// ---- Sound --------------------------------------------------------------------

enum Feed {
    Start { snapshot: Arc<Snapshot>, pos_us: i64 },
    Stop,
    Quit,
}

/// Frames mixed per step: 10 ms, so a cut lands within 10 ms of where it is drawn.
const FEED_FRAMES: usize = OUT_RATE as usize / 100;
/// How much sound to keep queued ahead of the speaker.
const FEED_AHEAD_FRAMES: usize = OUT_RATE as usize / 5;
const MAX_OPEN_READERS: usize = 4;

/// Mixes the audio tracks and keeps the output queue topped up while playing.
fn feed_audio(rx: Receiver<Feed>, out: Arc<AudioOutput>) {
    let mut readers: Vec<(Uuid, AudioReader)> = Vec::new();
    let mut job: Option<(Arc<Snapshot>, i64)> = None;
    let mut mix = vec![0.0f32; FEED_FRAMES * OUT_CHANNELS];
    let mut part = vec![0.0f32; FEED_FRAMES * OUT_CHANNELS];
    loop {
        // Idle: wait for work. Playing: look for a command without waiting.
        let cmd = if job.is_none() {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        } else {
            match rx.try_recv() {
                Ok(c) => Some(c),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        };
        match cmd {
            Some(Feed::Start { snapshot, pos_us }) => {
                readers.retain(|(id, _)| snapshot.audio_sources.contains_key(id));
                job = Some((snapshot, pos_us));
                continue;
            }
            Some(Feed::Stop) => {
                job = None;
                continue;
            }
            Some(Feed::Quit) => return,
            None => {}
        }
        let Some((snapshot, pos_us)) = job.as_mut() else { continue };
        if out.clear_pending() || out.queued_frames() >= FEED_AHEAD_FRAMES {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }

        mix.fill(0.0);
        for (clips, track_volume) in &snapshot.audio {
            let Some(clip) = clips.iter().find(|c| c.contains_point(*pos_us)) else { continue };
            if clip.properties.muted || !clip.properties.enabled {
                continue;
            }
            let Some(path) = snapshot.audio_sources.get(&clip.source_id) else { continue };
            let Some(reader) = reader_for(&mut readers, clip.source_id, path) else { continue };
            reader.read(clip.source_offset_at(*pos_us), &mut part);
            let volume = clip.properties.volume.max(0.0) * track_volume.max(0.0) * clip.fade_factor(*pos_us);
            let pan = clip.properties.pan.clamp(-1.0, 1.0);
            let (left, right) = (volume * (1.0 - pan.max(0.0)), volume * (1.0 + pan.min(0.0)));
            for (m, p) in mix.chunks_exact_mut(2).zip(part.chunks_exact(2)) {
                m[0] += p[0] * left;
                m[1] += p[1] * right;
            }
        }
        for sample in &mut mix {
            *sample = sample.clamp(-1.0, 1.0);
        }
        out.push(&mix);
        *pos_us += (FEED_FRAMES as i64 * 1_000_000) / OUT_RATE as i64;
    }
}

fn reader_for<'a>(readers: &'a mut Vec<(Uuid, AudioReader)>, id: Uuid, path: &PathBuf) -> Option<&'a mut AudioReader> {
    if let Some(idx) = readers.iter().position(|(r, _)| *r == id) {
        let entry = readers.remove(idx);
        readers.push(entry);
    } else {
        match AudioReader::open(path) {
            Ok(r) => {
                if readers.len() >= MAX_OPEN_READERS {
                    readers.remove(0);
                }
                readers.push((id, r));
            }
            Err(e) => {
                tracing::debug!("no sound from {}: {e}", path.display());
                return None;
            }
        }
    }
    readers.last_mut().map(|(_, r)| r)
}
