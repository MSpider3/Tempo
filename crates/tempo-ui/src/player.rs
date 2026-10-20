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
use tempo_timeline::{frame_to_us, Clip, ClipProperties, ClipType, Project, RationalFps, TitleData, TrackKind};
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
    /// The original files. Sound is read from the proxy in `sources` when it has a sound
    /// track (one file to read), and from here otherwise.
    audio_sources: HashMap<Uuid, PathBuf>,
    pub duration_us: i64,
    /// The frame grid playback steps on.
    pub fps: RationalFps,
}

impl Snapshot {
    /// `use_proxies` picks the proxy of a source when it has one.
    pub fn from_project(project: &Project, use_proxies: bool) -> Self {
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
                    (Some(p), true) if use_proxies && p.exists() => p.clone(),
                    _ => s.path.clone(),
                };
                (*id, path)
            })
            .collect();
        let audio = audible_tracks(&project.timeline)
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
            fps: project.fps,
        }
    }

    pub fn empty() -> Self {
        Self { tracks: Vec::new(), audio: Vec::new(), sources: HashMap::new(), audio_sources: HashMap::new(), duration_us: 0, fps: RationalFps::default() }
    }

    /// A one-clip snapshot used to preview a Media Pool item.
    pub fn from_source(id: Uuid, path: PathBuf, duration_us: i64, fps: RationalFps, has_video: bool, has_audio: bool) -> Self {
        let clip = |kind| Clip::new(Uuid::nil(), id, kind, "", 0, duration_us, 0, duration_us);
        Self {
            tracks: if has_video { vec![vec![clip(ClipType::Video)]] } else { Vec::new() },
            audio: if has_audio { vec![(vec![clip(ClipType::Audio)], 1.0)] } else { Vec::new() },
            audio_sources: HashMap::from([(id, path.clone())]),
            sources: HashMap::from([(id, path)]),
            duration_us,
            fps,
        }
    }
}

enum Cmd {
    Timeline(Arc<Snapshot>),
    Source(Option<Arc<Snapshot>>),
    Seek { us: i64, exact: bool },
    /// Move the timeline playhead, even while a source clip is being shown.
    SeekTimeline(i64),
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
    /// Where the timeline playhead is. Differs from `position_us` while a
    /// source clip is shown, which has a position of its own.
    timeline_position_us: AtomicI64,
    showing_source: AtomicBool,
    duration_us: AtomicI64,
    playing: AtomicBool,
    serial: AtomicU64,
    dropped: AtomicU32,
    /// Playback has switched to cheaper decoding because it could not keep up.
    degraded: AtomicBool,
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
        self.shared.showing_source.store(snapshot.is_some(), Ordering::Release);
        self.send(Cmd::Source(snapshot.map(Arc::new)));
    }

    /// `exact = false` shows the nearest keyframe, which is what a drag wants.
    pub fn seek(&self, us: i64, exact: bool) {
        let us = us.max(0);
        self.shared.position_us.store(us, Ordering::Release);
        if !self.shared.showing_source.load(Ordering::Acquire) {
            self.shared.timeline_position_us.store(us, Ordering::Release);
        }
        self.send(Cmd::Seek { us, exact });
    }

    /// Move the timeline playhead. While a source clip is shown, the viewer
    /// stays on the source and the timeline is where it was put on return.
    pub fn seek_timeline(&self, us: i64) {
        let us = us.max(0);
        self.shared.timeline_position_us.store(us, Ordering::Release);
        if !self.shared.showing_source.load(Ordering::Acquire) {
            self.shared.position_us.store(us, Ordering::Release);
        }
        self.send(Cmd::SeekTimeline(us));
    }

    /// The timeline playhead: where edits happen. Not the position inside a
    /// source clip that is being looked at.
    pub fn timeline_position_us(&self) -> i64 {
        self.shared.timeline_position_us.load(Ordering::Acquire)
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

    /// True while playback decodes at reduced quality to keep up.
    pub fn is_degraded(&self) -> bool {
        self.shared.degraded.load(Ordering::Relaxed)
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
    /// Open decoders, least recently used first, with the file each one reads.
    decoders: Vec<(Uuid, PathBuf, FfmpegDecoder)>,
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
    overload: Overload,
}

/// Notices playback that keeps missing its frame time, so decoding can get cheaper.
#[derive(Default)]
struct Overload {
    streak: u8,
    degraded: bool,
}

impl Overload {
    /// Record how long one frame took. True when playback should switch to cheaper
    /// decoding now: three frames in a row took over 1.25 times their time.
    fn record(&mut self, spent: Duration, frame: Duration) -> bool {
        if self.degraded {
            return false;
        }
        self.streak = if spent.as_secs_f64() > frame.as_secs_f64() * 1.25 { self.streak + 1 } else { 0 };
        self.degraded = self.streak >= 3;
        self.degraded
    }
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
            overload: Overload::default(),
            rx,
            shared,
            timeline: Arc::new(Snapshot::empty()),
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
        // Every start and stop of playback returns to full quality.
        self.set_degraded(false);
        self.playing = playing;
        self.shared.playing.store(playing, Ordering::Release);
        self.restart_clock();
    }

    /// Height pictures are decoded at: the preview quality, halved while degraded.
    fn output_height(&self) -> u32 {
        match (self.overload.degraded, self.max_height) {
            (false, h) => h,
            (true, 0) => 540,
            (true, h) => (h / 2).max(270),
        }
    }

    /// Switch cheaper decoding on or off for every open decoder. Turning it off redraws
    /// the current picture at full quality.
    fn set_degraded(&mut self, on: bool) {
        if !on {
            self.overload = Overload::default();
        }
        // `overload` says what is wanted; the shared flag says what the decoders were told.
        if self.shared.degraded.swap(on, Ordering::Relaxed) == on {
            return;
        }
        let height = self.output_height();
        for (_, _, d) in &mut self.decoders {
            d.set_fast_decode(on);
            d.set_max_output_height(height);
        }
        if on {
            tracing::info!("playback can't keep up; decoding at reduced quality until it stops");
        } else {
            self.exact = true;
            self.dirty = true;
            self.last_shown.clear();
        }
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
                // Drop decoders for files that are no longer referenced, and those
                // whose source now plays from another file (a proxy became ready,
                // or the preview quality changed).
                self.decoders.retain(|(id, path, _)| s.sources.get(id).map_or(self.source.is_some(), |now| now == path));
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
                if exact {
                    self.set_degraded(false);
                }
                self.pos_us = us;
                self.exact = exact;
                self.dirty = true;
                self.sync_audio();
            }
            Cmd::SeekTimeline(us) => {
                if self.source.is_some() {
                    self.timeline_pos = us;
                } else {
                    self.pos_us = us;
                    self.exact = true;
                    self.dirty = true;
                    self.sync_audio();
                }
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
                let height = self.output_height();
                for (_, _, d) in &mut self.decoders {
                    d.set_max_output_height(height);
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
            let timeline_pos = if self.source.is_some() { self.timeline_pos } else { self.pos_us };
            self.shared.timeline_position_us.store(timeline_pos, Ordering::Release);

            let started = Instant::now();
            self.render(&snap);
            self.dirty = false;

            if self.playing {
                let frame = Duration::from_micros((snap.fps.frame_duration_us().max(1) as f64 / self.speed.abs().max(1.0)) as u64);
                let spent = started.elapsed();
                if self.overload.record(spent, frame) {
                    self.set_degraded(true);
                }
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
            .map(|c| (c, source_request_us(c, pos, snap.fps)))
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
        if let Some(idx) = self.decoders.iter().position(|(d, open, _)| *d == id && open == path) {
            // Move to the back: most recently used.
            let entry = self.decoders.remove(idx);
            self.decoders.push(entry);
        } else {
            match FfmpegDecoder::open(path) {
                Ok(mut d) => {
                    d.set_max_output_height(self.output_height());
                    d.set_fast_decode(self.overload.degraded);
                    if self.decoders.len() >= MAX_OPEN_DECODERS {
                        self.decoders.remove(0);
                    }
                    self.decoders.push((id, path.clone(), d));
                }
                Err(e) => {
                    tracing::warn!("cannot open {}: {e}", path.display());
                    return None;
                }
            }
        }
        self.decoders.last_mut().map(|(_, _, d)| d)
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

/// Resample interleaved stereo `src` to fill `out`, by straight-line interpolation.
/// Used for retimed clips: reading `speed` times as much sound into one block plays it
/// faster (or slower), with the pitch moving too, as in Resolve on Linux and Windows.
fn stretch(src: &[f32], out: &mut [f32]) {
    let (n, m) = (src.len() / OUT_CHANNELS, out.len() / OUT_CHANNELS);
    if n == 0 {
        out.fill(0.0);
        return;
    }
    let step = n as f64 / m.max(1) as f64;
    for (i, frame) in out.chunks_exact_mut(OUT_CHANNELS).enumerate() {
        let x = i as f64 * step;
        let (j, t) = (x as usize, (x - x.floor()) as f32);
        let (a, b) = (j.min(n - 1), (j + 1).min(n - 1));
        for (c, sample) in frame.iter_mut().enumerate() {
            *sample = src[a * OUT_CHANNELS + c] * (1.0 - t) + src[b * OUT_CHANNELS + c] * t;
        }
    }
}

/// Mixes the audio tracks and keeps the output queue topped up while playing.
fn feed_audio(rx: Receiver<Feed>, out: Arc<AudioOutput>) {
    let mut readers: Vec<(Uuid, PathBuf, AudioReader)> = Vec::new();
    let mut job: Option<(Arc<Snapshot>, i64)> = None;
    let mut mix = vec![0.0f32; FEED_FRAMES * OUT_CHANNELS];
    let mut part = vec![0.0f32; FEED_FRAMES * OUT_CHANNELS];
    // Room for a block of a clip at the fastest speed, allocated once.
    let mut source = vec![0.0f32; (FEED_FRAMES as f64 * tempo_timeline::MAX_SPEED).ceil() as usize * OUT_CHANNELS];
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
                // Keep readers of the file each source should now be read from.
                readers.retain(|(id, path, _)| snapshot.sources.get(id).or(snapshot.audio_sources.get(id)) == Some(path));
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
            let original = snapshot.audio_sources.get(&clip.source_id);
            let Some(preferred) = snapshot.sources.get(&clip.source_id).or(original) else { continue };
            let Some(reader) = reader_for(&mut readers, clip.source_id, preferred, original) else { continue };
            let speed = clip.properties.speed;
            if speed == 1.0 {
                reader.read(clip.source_offset_at(*pos_us), &mut part);
            } else {
                let frames = ((FEED_FRAMES as f64 * speed).round() as usize).clamp(1, source.len() / OUT_CHANNELS);
                let block = &mut source[..frames * OUT_CHANNELS];
                reader.read(clip.source_offset_at(*pos_us), block);
                stretch(block, &mut part);
            }
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

/// The open reader for a source, opening `preferred` (or, if it has no sound, `fallback`).
fn reader_for<'a>(readers: &'a mut Vec<(Uuid, PathBuf, AudioReader)>, id: Uuid, preferred: &PathBuf, fallback: Option<&PathBuf>) -> Option<&'a mut AudioReader> {
    if let Some(idx) = readers.iter().position(|(r, _, _)| *r == id) {
        let entry = readers.remove(idx);
        readers.push(entry);
    } else {
        let opened = std::iter::once(preferred).chain(fallback.filter(|f| *f != preferred)).find_map(|path| match AudioReader::open(path) {
            Ok(r) => Some((path.clone(), r)),
            Err(e) => {
                tracing::debug!("no sound from {}: {e}", path.display());
                None
            }
        });
        let (path, reader) = opened?;
        if readers.len() >= MAX_OPEN_READERS {
            readers.remove(0);
        }
        readers.push((id, path, reader));
    }
    readers.last_mut().map(|(_, _, r)| r)
}

/// The audio tracks playback hears: those not muted, and only the soloed ones when any
/// track is soloed. Solo is for listening only; export does not look at it.
fn audible_tracks(timeline: &tempo_timeline::Timeline) -> impl Iterator<Item = &tempo_timeline::Track> {
    let audio = || timeline.tracks.iter().filter(|t| t.kind == TrackKind::Audio && t.enabled);
    let any_solo = audio().any(|t| t.solo);
    audio().filter(move |t| !any_solo || t.solo)
}

/// Source time to decode for the picture shown at timeline position `pos`: the start of
/// the project frame that holds `pos`, mapped into the clip and never before its In point.
/// The same timeline frame always asks for the same source time, so it is decoded once.
fn source_request_us(clip: &Clip, pos: i64, fps: RationalFps) -> i64 {
    if fps.num == 0 || fps.den == 0 {
        return clip.source_offset_at(pos);
    }
    let frame = (pos.max(0) as i128 * fps.num as i128 / (1_000_000 * fps.den as i128)) as i64;
    clip.source_offset_at(frame_to_us(frame, fps).max(clip.timeline_in))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(timeline_in: i64, source_in: i64) -> Clip {
        Clip::new(Uuid::nil(), Uuid::nil(), ClipType::Video, "c", timeline_in, timeline_in + 5_000_000, source_in, source_in + 5_000_000)
    }

    const FRAME: Duration = Duration::from_millis(33);

    #[test]
    fn overload_enters_after_three_consecutive_slow_frames() {
        let mut o = Overload::default();
        let slow = FRAME * 2;
        assert!(!o.record(slow, FRAME));
        assert!(!o.record(slow, FRAME));
        assert!(o.record(slow, FRAME));
        // Entered once; further slow frames don't ask again.
        assert!(!o.record(slow, FRAME));
        assert!(o.degraded);
    }

    #[test]
    fn one_fast_frame_resets_the_streak() {
        let mut o = Overload::default();
        o.record(FRAME * 2, FRAME);
        o.record(FRAME * 2, FRAME);
        o.record(FRAME, FRAME);
        assert!(!o.record(FRAME * 2, FRAME));
        assert!(!o.record(FRAME * 2, FRAME));
        assert!(o.record(FRAME * 2, FRAME));
    }

    #[test]
    fn slightly_slow_frames_are_tolerated() {
        let mut o = Overload::default();
        for _ in 0..10 {
            assert!(!o.record(FRAME * 6 / 5, FRAME)); // 1.2x is within 1.25x
        }
    }

    #[test]
    fn stretch_fills_the_block_and_reads_the_source_at_speed() {
        // A ramp of 8 stereo frames read at double speed into 4 frames: every other one.
        let src: Vec<f32> = (0..8).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let mut out = vec![9.0f32; 4 * 2];
        stretch(&src, &mut out);
        assert_eq!(out, [0.0, -0.0, 2.0, -2.0, 4.0, -4.0, 6.0, -6.0]);
        // Half speed: 2 source frames spread over 4, in between values interpolated.
        let mut slow = vec![0.0f32; 4 * 2];
        stretch(&[0.0, 0.0, 1.0, 1.0], &mut slow);
        assert_eq!(slow, [0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn solo_plays_only_soloed_audio_tracks() {
        let mut timeline = tempo_timeline::Timeline::new_default();
        let names = |t: &tempo_timeline::Timeline| audible_tracks(t).map(|t| t.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&timeline), ["Audio 1", "Audio 2"]);
        let a2 = timeline.tracks.iter_mut().find(|t| t.name == "Audio 2").unwrap();
        a2.solo = true;
        assert_eq!(names(&timeline), ["Audio 2"]);
        // A muted track stays silent even when soloed.
        timeline.tracks.iter_mut().find(|t| t.name == "Audio 2").unwrap().enabled = false;
        assert_eq!(names(&timeline), ["Audio 1"]);
    }

    #[test]
    fn first_timeline_frame_never_requests_before_source_in() {
        // Rounding the source offset on the project grid used to ask for 33 333 here.
        let c = clip(0, 50_000);
        assert_eq!(source_request_us(&c, 0, RationalFps::FPS_30), 50_000);
        assert_eq!(source_request_us(&c, 40_000, RationalFps::FPS_30), 50_000 + 33_333);
    }

    #[test]
    fn clip_starting_between_frames_starts_at_its_in_point() {
        let c = clip(110_000, 2_000_000);
        for fps in [RationalFps::FPS_24, RationalFps::FPS_30, RationalFps::FPS_60] {
            assert_eq!(source_request_us(&c, 110_000, fps), 2_000_000);
        }
    }

    #[test]
    fn ntsc_rates_stay_on_the_rational_grid_after_an_hour() {
        let fps = RationalFps::FPS_29_97;
        let c = clip(0, 0);
        let pos = 3_600_000_000;
        let frame = (pos as i128 * 30_000 / 1_001_000_000) as i64;
        assert_eq!(source_request_us(&c, pos, fps), frame_to_us(frame, fps));
        // Every position inside one frame asks for the same time.
        let start = frame_to_us(frame, fps);
        assert_eq!(source_request_us(&c, start + 20_000, fps), start);
    }
}
