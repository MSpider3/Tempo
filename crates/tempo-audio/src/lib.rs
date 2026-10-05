//! tempo-audio: sound output through PipeWire.
//!
//! Samples are pushed into a lock-free queue by a feeder thread. PipeWire's
//! real-time callback only pops from that queue and updates a few atomics: it
//! never allocates, locks or blocks. The number of frames it has played is the
//! clock the picture follows.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam::queue::ArrayQueue;
use pipewire as pw;
use pw::{properties::properties, spa};

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// Room for two seconds of sound.
const QUEUE_SAMPLES: usize = RATE as usize * CHANNELS * 2;

struct Shared {
    queue: ArrayQueue<f32>,
    /// Frames played since the last `clear`.
    played: AtomicU64,
    /// Set by `clear`; the callback empties the queue and resets `played`.
    flush: AtomicBool,
    /// Peak level of the last callback per channel, as `f32` bits.
    peak: [AtomicU32; CHANNELS],
}

pub struct AudioOutput {
    shared: Arc<Shared>,
    quit: pw::channel::Sender<()>,
}

impl AudioOutput {
    /// Connect to PipeWire. `None` if there is no sound server; the caller then
    /// plays silently on a wall clock.
    pub fn start() -> Option<Self> {
        let shared = Arc::new(Shared {
            queue: ArrayQueue::new(QUEUE_SAMPLES),
            played: AtomicU64::new(0),
            flush: AtomicBool::new(false),
            peak: [AtomicU32::new(0), AtomicU32::new(0)],
        });
        let (quit, quit_rx) = pw::channel::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<bool>();
        let thread_shared = shared.clone();
        let spawned = std::thread::Builder::new().name("tempo-audio-out".into()).spawn(move || {
            if let Err(e) = run(thread_shared, quit_rx, &ready_tx) {
                tracing::warn!("audio output unavailable: {e}");
                let _ = ready_tx.send(false);
            }
        });
        if spawned.is_err() {
            return None;
        }
        match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(true) => Some(Self { shared, quit }),
            _ => None,
        }
    }

    /// Queue interleaved stereo samples. Returns how many were accepted.
    pub fn push(&self, samples: &[f32]) -> usize {
        let mut n = 0;
        for s in samples {
            if self.shared.queue.push(*s).is_err() {
                break;
            }
            n += 1;
        }
        n
    }

    pub fn queued_frames(&self) -> usize {
        self.shared.queue.len() / CHANNELS
    }

    /// Drop everything queued and restart the frame count at zero.
    pub fn clear(&self) {
        self.shared.flush.store(true, Ordering::Release);
    }

    /// True until the audio thread has carried out the last `clear`.
    pub fn clear_pending(&self) -> bool {
        self.shared.flush.load(Ordering::Acquire)
    }

    pub fn frames_played(&self) -> u64 {
        self.shared.played.load(Ordering::Acquire)
    }

    /// Peak level (0.0–1.0) of the most recent block, left and right.
    pub fn peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.shared.peak[0].load(Ordering::Relaxed)),
            f32::from_bits(self.shared.peak[1].load(Ordering::Relaxed)),
        )
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        let _ = self.quit.send(());
    }
}

fn run(shared: Arc<Shared>, quit_rx: pw::channel::Receiver<()>, ready: &std::sync::mpsc::Sender<bool>) -> Result<(), pw::Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let loop_for_quit = mainloop.clone();
    let _quit = quit_rx.attach(mainloop.loop_(), move |_| loop_for_quit.quit());

    let stream = pw::stream::StreamBox::new(
        &core,
        "Tempo",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_ROLE => "Movie",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::NODE_NAME => "Tempo",
            // About 21 ms per block: low enough for lip sync, easy on a slow CPU.
            *pw::keys::NODE_LATENCY => "1024/48000",
        },
    )?;

    const STRIDE: usize = std::mem::size_of::<f32>() * CHANNELS;
    let _listener = stream
        .add_local_listener_with_user_data(shared)
        .process(|stream, shared| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let requested = buffer.requested() as usize;
            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else { return };
            let frames = {
                let Some(slice) = data.data() else { return };
                let capacity = slice.len() / STRIDE;
                let frames = if requested > 0 { requested.min(capacity) } else { capacity };

                if shared.flush.swap(false, Ordering::AcqRel) {
                    while shared.queue.pop().is_some() {}
                    shared.played.store(0, Ordering::Release);
                }

                let mut peak = [0.0f32; CHANNELS];
                let mut played = 0u64;
                for (i, sample) in slice[..frames * STRIDE].chunks_exact_mut(4).enumerate() {
                    // Count a frame only when its first sample was really there.
                    let value = shared.queue.pop();
                    if i % CHANNELS == 0 && value.is_some() {
                        played += 1;
                    }
                    let value = value.unwrap_or(0.0);
                    peak[i % CHANNELS] = peak[i % CHANNELS].max(value.abs());
                    sample.copy_from_slice(&value.to_le_bytes());
                }
                shared.played.fetch_add(played, Ordering::AcqRel);
                for (slot, p) in shared.peak.iter().zip(peak) {
                    slot.store(p.to_bits(), Ordering::Relaxed);
                }
                frames
            };
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = STRIDE as _;
            *chunk.size_mut() = (STRIDE * frames) as _;
        })
        .register()?;

    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(RATE);
    info.set_channels(CHANNELS as u32);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    let object = spa::pod::Object {
        type_: spa::sys::SPA_TYPE_OBJECT_Format,
        id: spa::sys::SPA_PARAM_EnumFormat,
        properties: info.into(),
    };
    let bytes = spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &spa::pod::Value::Object(object))
        .map_err(|_| pw::Error::CreationFailed)?
        .0
        .into_inner();
    let pod = spa::pod::Pod::from_bytes(&bytes).ok_or(pw::Error::CreationFailed)?;

    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut [pod],
    )?;

    let _ = ready.send(true);
    mainloop.run();
    Ok(())
}
