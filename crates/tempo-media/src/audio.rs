//! Audio decoding. Each reader has its own demuxer, so reading sound never
//! disturbs the video decoder of the same file.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use ffmpeg_next::format;
use ffmpeg_next::media::Type as FfmpegMediaType;
use ffmpeg_next::software::resampling::context::Context as Resampler;
use ffmpeg_next::{ChannelLayout, Rational};

use crate::decoder::to_us;
use crate::error::{MediaError, Result};
use crate::probe::ensure_ffmpeg_init;

/// Everything is converted to this: 48 kHz, stereo, interleaved `f32`.
pub const OUT_RATE: u32 = 48_000;
pub const OUT_CHANNELS: usize = 2;

/// A jump further ahead than this seeks instead of decoding through.
const SEEK_AHEAD_FRAMES: i64 = OUT_RATE as i64;

pub struct AudioReader {
    ictx: format::context::Input,
    stream_index: usize,
    decoder: ffmpeg_next::decoder::Audio,
    time_base: Rational,
    start_us: i64,
    resampler: Resampler,
    /// Decoded samples waiting to be read, interleaved.
    fifo: VecDeque<f32>,
    /// Position of the front of `fifo`, in output frames from the start of the file.
    fifo_frame: i64,
    /// False until the first frame after a seek has told us where we are.
    positioned: bool,
    eof: bool,
}

// See `FfmpegDecoder`: used by one thread at a time.
unsafe impl Send for AudioReader {}

impl AudioReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        ensure_ffmpeg_init();
        let path = path.as_ref();
        let mut ictx = format::input(&path).map_err(MediaError::Ffmpeg)?;
        let stream = ictx
            .streams()
            .best(FfmpegMediaType::Audio)
            .ok_or_else(|| MediaError::StreamNotFound("No audio stream found".to_string()))?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let start_us = match stream.start_time() {
            t if t == ffmpeg_next::ffi::AV_NOPTS_VALUE => 0,
            t => to_us(t, time_base),
        };
        let decoder = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())
            .map_err(MediaError::Ffmpeg)?
            .decoder()
            .audio()
            .map_err(MediaError::Ffmpeg)?;
        // Some files (mono phone recordings) carry a channel count but no layout.
        let layout = if decoder.channel_layout().is_empty() {
            ChannelLayout::default(decoder.channels() as i32)
        } else {
            decoder.channel_layout()
        };
        let resampler = Resampler::get(
            decoder.format(),
            layout,
            decoder.rate(),
            ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
            ChannelLayout::STEREO,
            OUT_RATE,
        )
        .map_err(MediaError::Ffmpeg)?;

        for mut s in ictx.streams_mut() {
            if s.index() != stream_index {
                // SAFETY: the stream belongs to `ictx`, which is alive and not shared.
                unsafe { (*s.as_mut_ptr()).discard = ffmpeg_next::ffi::AVDiscard::AVDISCARD_ALL };
            }
        }

        Ok(Self {
            ictx,
            stream_index,
            decoder,
            time_base,
            start_us,
            resampler,
            fifo: VecDeque::new(),
            fifo_frame: 0,
            positioned: false,
            eof: false,
        })
    }

    /// True once the file has ended and everything decoded has been read.
    pub fn finished(&self) -> bool {
        self.eof && self.fifo.is_empty()
    }

    fn seek(&mut self, us: i64) {
        let ts = (us + self.start_us).max(0);
        let _ = self.ictx.seek(ts, ..ts);
        self.decoder.flush();
        self.fifo.clear();
        self.positioned = false;
        self.eof = false;
    }

    /// Decode one more packet's worth into the fifo. Returns false at end of file.
    fn decode_more(&mut self) -> bool {
        if self.eof {
            return false;
        }
        let mut packet = ffmpeg_next::Packet::empty();
        loop {
            match packet.read(&mut self.ictx) {
                Ok(()) if packet.stream() == self.stream_index => break,
                Ok(()) => continue,
                Err(ffmpeg_next::Error::Eof) => {
                    self.eof = true;
                    let _ = self.decoder.send_eof();
                    break;
                }
                Err(_) => continue,
            }
        }
        if !self.eof && self.decoder.send_packet(&packet).is_err() {
            return true;
        }
        let mut decoded = ffmpeg_next::frame::Audio::empty();
        let mut resampled = ffmpeg_next::frame::Audio::empty();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            if !self.positioned {
                let pts_us = to_us(decoded.pts().unwrap_or(0), self.time_base) - self.start_us;
                self.fifo_frame = pts_us * OUT_RATE as i64 / 1_000_000;
                self.positioned = true;
            }
            if self.resampler.run(&decoded, &mut resampled).is_err() {
                continue;
            }
            let count = resampled.samples() * OUT_CHANNELS;
            let bytes = resampled.data(0);
            self.fifo.extend(
                bytes
                    .as_chunks::<4>().0.iter()
                    .take(count)
                    .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])),
            );
        }
        !self.eof
    }

    /// Fill `out` (interleaved stereo) with the sound that starts `start_us` into
    /// the file. Anything the file does not have is left silent.
    pub fn read(&mut self, start_us: i64, out: &mut [f32]) {
        out.fill(0.0);
        let want = start_us.max(0) * OUT_RATE as i64 / 1_000_000;
        let buffered = (self.fifo.len() / OUT_CHANNELS) as i64;
        if !self.positioned || want < self.fifo_frame || want > self.fifo_frame + buffered + SEEK_AHEAD_FRAMES {
            self.seek(start_us);
        }

        // Find our position, then drop what lies before the wanted frame.
        while !self.positioned && self.decode_more() {}
        loop {
            let skip = (want - self.fifo_frame).max(0) as usize * OUT_CHANNELS;
            if skip == 0 {
                break;
            }
            let n = skip.min(self.fifo.len());
            self.fifo.drain(..n);
            self.fifo_frame += (n / OUT_CHANNELS) as i64;
            if n < skip && !self.decode_more() && self.fifo.is_empty() {
                return;
            }
        }

        // The file may start later than asked (a gap): leave that part silent.
        let lead = ((self.fifo_frame - want).max(0) as usize * OUT_CHANNELS).min(out.len());
        let needed = out.len() - lead;
        while self.fifo.len() < needed && self.decode_more() {}
        let n = needed.min(self.fifo.len());
        for (dst, src) in out[lead..lead + n].iter_mut().zip(self.fifo.drain(..n)) {
            *dst = src;
        }
        self.fifo_frame += (n / OUT_CHANNELS) as i64;
    }
}

/// Loudest sample in each slice of the file, as 0–255, `per_second` values for
/// every second of sound. Blocking; reads the whole file.
pub fn waveform_peaks(path: &Path, per_second: u32, cancel: &AtomicBool) -> Result<Vec<u8>> {
    let mut reader = AudioReader::open(path)?;
    let frames_per_peak = (OUT_RATE / per_second.max(1)).max(1) as usize;
    let mut chunk = vec![0.0f32; OUT_RATE as usize * OUT_CHANNELS];
    let mut peaks = Vec::new();
    let mut second = 0i64;
    while !reader.finished() {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::AudioDecodeFailed);
        }
        reader.read(second * 1_000_000, &mut chunk);
        for slice in chunk.chunks(frames_per_peak * OUT_CHANNELS) {
            let peak = slice.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            peaks.push((peak.min(1.0) * 255.0) as u8);
        }
        second += 1;
    }
    Ok(peaks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample(name: &str) -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/media").join(name);
        p.exists().then_some(p)
    }

    #[test]
    fn reads_continuously_without_gaps_or_repeats() {
        let Some(path) = sample("sample_10s_sync.mp4") else { return };
        // Reading one long block must equal reading the same span in small blocks.
        let mut whole = vec![0.0f32; 9600 * 2];
        AudioReader::open(&path).expect("open").read(1_000_000, &mut whole);

        let mut reader = AudioReader::open(&path).expect("open");
        let mut pieces = Vec::new();
        for i in 0..20 {
            let mut part = vec![0.0f32; 480 * 2];
            reader.read(1_000_000 + i * 10_000, &mut part);
            pieces.extend(part);
        }
        assert_eq!(whole.len(), pieces.len());
        let diff = whole.iter().zip(&pieces).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(diff < 1e-4, "largest difference {diff}");
        assert!(whole.iter().any(|s| s.abs() > 0.001), "the sample has sound");
    }

    #[test]
    fn seeking_back_gives_the_same_samples() {
        let Some(path) = sample("sample_10s_sync.mp4") else { return };
        let mut reader = AudioReader::open(&path).expect("open");
        let mut first = vec![0.0f32; 4800];
        reader.read(2_000_000, &mut first);
        let mut elsewhere = vec![0.0f32; 4800];
        reader.read(6_000_000, &mut elsewhere);
        let mut again = vec![0.0f32; 4800];
        reader.read(2_000_000, &mut again);
        let diff = first.iter().zip(&again).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(diff < 0.02, "largest difference {diff}");
    }

    #[test]
    fn past_the_end_is_silence_and_peaks_cover_the_file() {
        let Some(path) = sample("sample_10s_sync.mp4") else { return };
        let mut reader = AudioReader::open(&path).expect("open");
        let mut out = vec![1.0f32; 960];
        reader.read(3_600_000_000, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));

        let peaks = waveform_peaks(&path, 50, &AtomicBool::new(false)).expect("peaks");
        // About ten seconds at 50 peaks a second.
        assert!((450..=650).contains(&peaks.len()), "{}", peaks.len());
        assert!(peaks.iter().any(|p| *p > 0));
    }
}
