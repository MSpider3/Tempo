//! Video decoding: one open file, decoded on demand.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use ffmpeg_next::format::{self, Pixel};
use ffmpeg_next::media::Type as FfmpegMediaType;
use ffmpeg_next::software::scaling::{context::Context as ScalerContext, flag::Flags};
use ffmpeg_next::{ffi, Rational};

use crate::error::{MediaError, Result};
use crate::probe::ensure_ffmpeg_init;

// ---- Hardware decoding (VA-API) -------------------------------------------------

static HARDWARE_DECODE: AtomicBool = AtomicBool::new(false);

/// Turn hardware decoding on or off for files opened from now on. When it is on
/// but the graphics driver cannot be used, decoding quietly stays on the processor.
pub fn set_hardware_decode(on: bool) {
    HARDWARE_DECODE.store(on, Ordering::Relaxed);
}

/// The VA-API device, opened once and shared by every decoder.
struct HwDevice(*mut ffi::AVBufferRef);
// SAFETY: FFmpeg's buffer references are reference-counted with atomics, and
// this one is only ever read (to make new references).
unsafe impl Send for HwDevice {}
unsafe impl Sync for HwDevice {}

fn hw_device() -> Option<&'static HwDevice> {
    static DEVICE: OnceLock<Option<HwDevice>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let mut device: *mut ffi::AVBufferRef = std::ptr::null_mut();
            // SAFETY: `device` is a valid out-pointer; null arguments ask for the default device.
            let result = unsafe {
                ffi::av_hwdevice_ctx_create(&mut device, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, std::ptr::null(), std::ptr::null_mut(), 0)
            };
            if result >= 0 && !device.is_null() {
                tracing::info!("hardware video decoding (VA-API) is available");
                Some(HwDevice(device))
            } else {
                tracing::info!("hardware video decoding is not available; decoding on the processor");
                None
            }
        })
        .as_ref()
}

/// Called by FFmpeg to choose the output format. Takes the VA-API surface format
/// when it is offered; otherwise the first format, which is ordinary software decoding.
unsafe extern "C" fn pick_format(_ctx: *mut ffi::AVCodecContext, formats: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    let mut p = formats;
    // SAFETY: FFmpeg passes a list terminated by AV_PIX_FMT_NONE.
    unsafe {
        while *p != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            if *p == ffi::AVPixelFormat::AV_PIX_FMT_VAAPI {
                return *p;
            }
            p = p.add(1);
        }
        *formats
    }
}

/// A forward jump shorter than this is decoded through; longer jumps seek.
pub const SEEK_THRESHOLD_US: i64 = 2_000_000;

/// One RGBA picture.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub pts_us: i64,
}

pub struct FfmpegDecoder {
    pub path: PathBuf,
    ictx: format::context::Input,
    stream_index: usize,
    decoder: ffmpeg_next::decoder::Video,
    time_base: Rational,
    /// Timestamp of the first frame; subtracted so every file starts at zero.
    start_us: i64,
    video_width: u32,
    video_height: u32,
    out_width: u32,
    out_height: u32,
    /// Converter to RGBA, built for the format frames actually arrive in (which
    /// differs between software and hardware decoding) and rebuilt if that changes.
    scaler: Option<(Pixel, ScalerContext)>,
    last_decoded_pts_us: i64,
    /// The request and result of the last exact decode. Asking again for a time
    /// that falls on the same frame returns this copy instead of reading on.
    last_exact: Option<(i64, VideoFrame)>,
}

// FFmpeg contexts may move between threads as long as only one thread uses them at a time.
unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        ensure_ffmpeg_init();
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(MediaError::FileNotFound(path.to_string_lossy().to_string()));
        }
        let mut ictx = format::input(&path).map_err(MediaError::Ffmpeg)?;

        let stream = ictx
            .streams()
            .best(FfmpegMediaType::Video)
            .ok_or_else(|| MediaError::StreamNotFound("No video stream found".to_string()))?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let start_us = match stream.start_time() {
            t if t == ffmpeg_next::ffi::AV_NOPTS_VALUE => 0,
            t => to_us(t, time_base),
        };

        let mut codec_ctx =
            ffmpeg_next::codec::context::Context::from_parameters(stream.parameters()).map_err(MediaError::Ffmpeg)?;
        codec_ctx.set_threading(ffmpeg_next::codec::threading::Config {
            count: 0,
            // Frame threading: most H.264/HEVC files have one slice per frame,
            // so slice threading would decode on a single core.
            kind: ffmpeg_next::codec::threading::Type::Frame,
        });
        if HARDWARE_DECODE.load(Ordering::Relaxed) {
            if let Some(device) = hw_device() {
                // SAFETY: the codec context is valid and not yet opened; it takes its own
                // reference to the device, which FFmpeg releases with the context.
                unsafe {
                    let ctx = codec_ctx.as_mut_ptr();
                    (*ctx).hw_device_ctx = ffi::av_buffer_ref(device.0);
                    (*ctx).get_format = Some(pick_format);
                }
            }
        }
        let decoder = codec_ctx.decoder().video().map_err(MediaError::Ffmpeg)?;
        let (video_width, video_height) = (decoder.width(), decoder.height());

        // Only the video stream is read here; skipping the rest saves demuxing work.
        for mut s in ictx.streams_mut() {
            if s.index() != stream_index {
                // SAFETY: the stream belongs to `ictx`, which is alive and not shared.
                unsafe { (*s.as_mut_ptr()).discard = ffmpeg_next::ffi::AVDiscard::AVDISCARD_ALL };
            }
        }

        Ok(Self {
            path,
            ictx,
            stream_index,
            decoder,
            time_base,
            start_us,
            video_width,
            video_height,
            out_width: video_width,
            out_height: video_height,
            scaler: None,
            last_decoded_pts_us: -1,
            last_exact: None,
        })
    }

    pub fn video_width(&self) -> u32 {
        self.video_width
    }

    pub fn video_height(&self) -> u32 {
        self.video_height
    }

    /// Limit the height of decoded frames. Frames are scaled straight from the
    /// decoder's pixel format to RGBA at the reduced size, so a smaller preview
    /// costs less conversion work and less memory. `0` restores full size.
    pub fn set_max_output_height(&mut self, max_height: u32) {
        let (w, h) = if max_height == 0 || max_height >= self.video_height || self.video_height == 0 {
            (self.video_width, self.video_height)
        } else {
            let w = (self.video_width as u64 * max_height as u64 / self.video_height as u64) as u32;
            ((w.max(2) + 1) & !1, (max_height.max(2) + 1) & !1)
        };
        if (w, h) != (self.out_width, self.out_height) {
            self.out_width = w;
            self.out_height = h;
            self.scaler = None;
            self.last_exact = None;
        }
    }

    fn scale_frame(&mut self, decoded: &ffmpeg_next::frame::Video, pts_us: i64) -> Result<VideoFrame> {
        // A hardware frame lives in graphics memory: copy it down first.
        let mut downloaded = ffmpeg_next::frame::Video::empty();
        let decoded = if decoded.format() == Pixel::VAAPI {
            // SAFETY: both frames are valid; FFmpeg allocates the destination's buffers.
            let result = unsafe { ffi::av_hwframe_transfer_data(downloaded.as_mut_ptr(), decoded.as_ptr(), 0) };
            if result < 0 {
                return Err(MediaError::FrameDecodeFailed);
            }
            &downloaded
        } else {
            decoded
        };

        let format = decoded.format();
        if self.scaler.as_ref().is_none_or(|(f, _)| *f != format) {
            let scaler = ScalerContext::get(format, decoded.width(), decoded.height(), Pixel::RGBA, self.out_width, self.out_height, Flags::FAST_BILINEAR)
                .map_err(MediaError::Ffmpeg)?;
            self.scaler = Some((format, scaler));
        }
        let mut rgba = ffmpeg_next::frame::Video::empty();
        match self.scaler.as_mut() {
            Some((_, scaler)) => scaler.run(decoded, &mut rgba).map_err(|_| MediaError::FrameDecodeFailed)?,
            None => return Err(MediaError::FrameDecodeFailed),
        }
        let (width, height) = (self.out_width, self.out_height);
        let data = rgba.data(0);
        let stride = rgba.stride(0);
        let row_bytes = (width * 4) as usize;
        let total = row_bytes * height as usize;
        let mut buffer = Vec::with_capacity(total);
        if stride == row_bytes && data.len() >= total {
            buffer.extend_from_slice(&data[..total]);
        } else {
            for y in 0..height as usize {
                let start = y * stride;
                if let Some(row) = data.get(start..start + row_bytes) {
                    buffer.extend_from_slice(row);
                }
            }
        }
        Ok(VideoFrame { width, height, data: buffer, pts_us })
    }

    /// Exact frame at or after `target_pts_us` (microseconds from the start of the file).
    pub fn decode_video_frame(&mut self, target_pts_us: i64) -> Result<VideoFrame> {
        if let Some((asked, frame)) = &self.last_exact {
            if target_pts_us >= *asked && target_pts_us <= frame.pts_us {
                return Ok(frame.clone());
            }
        }
        let frame = self.decode_video(target_pts_us, false)?;
        self.last_exact = Some((target_pts_us, frame.clone()));
        Ok(frame)
    }

    /// Nearest keyframe at or before `target_pts_us`. Much cheaper than an exact
    /// seek on long-GOP footage; used while the user is dragging the playhead.
    pub fn decode_keyframe(&mut self, target_pts_us: i64) -> Result<VideoFrame> {
        self.last_exact = None;
        self.decode_video(target_pts_us, true)
    }

    fn decode_video(&mut self, target_pts_us: i64, keyframe_only: bool) -> Result<VideoFrame> {
        let need_seek = keyframe_only
            || self.last_decoded_pts_us < 0
            || target_pts_us < self.last_decoded_pts_us
            || target_pts_us - self.last_decoded_pts_us > SEEK_THRESHOLD_US;
        if need_seek {
            let ts = target_pts_us + self.start_us;
            let _ = self.ictx.seek(ts, ..ts);
            self.decoder.flush();
            self.last_decoded_pts_us = -1;
        }

        let mut decoded = ffmpeg_next::frame::Video::empty();
        let mut last_before: Option<(ffmpeg_next::frame::Video, i64)> = None;
        // Some decoders (OpenH264, which Fedora's FFmpeg uses for H.264) still hand
        // out a frame from before the seek after being flushed. Once we know the
        // seek landed at or before the target, frames later than the target are
        // such leftovers until the first frame at or before it has come out.
        let mut stale_possible = false;
        let mut first_packet = need_seek;

        // Read packets one at a time so `self` stays free for scaling.
        loop {
            let mut packet = ffmpeg_next::Packet::empty();
            match packet.read(&mut self.ictx) {
                Ok(()) => {}
                Err(ffmpeg_next::Error::Eof) => break,
                Err(_) => continue,
            }
            if packet.stream() != self.stream_index {
                continue;
            }
            if first_packet {
                first_packet = false;
                let packet_us = packet.pts().or(packet.dts()).map(|t| to_us(t, self.time_base) - self.start_us);
                stale_possible = packet_us.is_some_and(|t| t <= target_pts_us);
            }
            if self.decoder.send_packet(&packet).is_err() {
                continue;
            }
            while self.decoder.receive_frame(&mut decoded).is_ok() {
                let pts_us = to_us(decoded.pts().unwrap_or(0), self.time_base) - self.start_us;
                if stale_possible {
                    if pts_us > target_pts_us {
                        continue;
                    }
                    stale_possible = false;
                }
                self.last_decoded_pts_us = pts_us;
                if keyframe_only || pts_us >= target_pts_us {
                    return self.scale_frame(&decoded, pts_us);
                }
                last_before = Some((decoded.clone(), pts_us));
            }
        }

        // End of file: drain the frames the decoder is still holding.
        let _ = self.decoder.send_eof();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            let pts_us = to_us(decoded.pts().unwrap_or(0), self.time_base) - self.start_us;
            if pts_us >= target_pts_us {
                self.last_decoded_pts_us = -1; // drained; the next call must seek
                return self.scale_frame(&decoded, pts_us);
            }
            last_before = Some((decoded.clone(), pts_us));
        }
        self.last_decoded_pts_us = -1;

        // Past the last frame: show the last one there is.
        match last_before {
            Some((raw, pts)) => self.scale_frame(&raw, pts),
            None => Err(MediaError::FrameDecodeFailed),
        }
    }
}

pub(crate) fn to_us(ts: i64, time_base: Rational) -> i64 {
    (ts as i128 * time_base.numerator() as i128 * 1_000_000 / time_base.denominator().max(1) as i128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/media").join(name);
        p.exists().then_some(p)
    }

    #[test]
    fn decodes_first_frame_at_full_and_reduced_size() {
        let Some(path) = sample("sample_1080p_h264.mp4") else { return };
        let mut d = FfmpegDecoder::open(path).expect("open");
        let f = d.decode_video_frame(0).expect("frame");
        assert_eq!((f.width, f.height, f.data.len()), (1920, 1080, 1920 * 1080 * 4));
        d.set_max_output_height(540);
        let f = d.decode_video_frame(0).expect("frame");
        assert_eq!((f.width, f.height, f.data.len()), (960, 540, 960 * 540 * 4));
    }

    #[test]
    fn asking_for_the_same_time_twice_returns_the_same_frame() {
        let Some(path) = sample("sample_1080p_h264.mp4") else { return };
        let mut d = FfmpegDecoder::open(path).expect("open");
        let a = d.decode_video_frame(1_000_000).expect("frame");
        let b = d.decode_video_frame(1_000_000).expect("frame");
        assert_eq!(a.pts_us, b.pts_us);
        assert!(a.pts_us >= 1_000_000 && a.pts_us < 1_100_000);
    }

    #[test]
    fn seeks_backward_and_reads_past_the_end() {
        let Some(path) = sample("sample_1080p_h264.mp4") else { return };
        let mut d = FfmpegDecoder::open(path).expect("open");
        let late = d.decode_video_frame(3_000_000).expect("frame");
        let early = d.decode_video_frame(500_000).expect("frame");
        assert!(early.pts_us < late.pts_us);
        // Far past the end still gives the last picture instead of an error.
        assert!(d.decode_video_frame(3_600_000_000).is_ok());
        // And the decoder is usable afterwards.
        assert!(d.decode_video_frame(0).is_ok());
    }

    #[test]
    fn hardware_decode_setting_never_breaks_decoding() {
        // With the setting on, a machine without a usable driver must still decode
        // (on the processor), and a machine with one must give the same picture size.
        let Some(path) = sample("sample_1080p_h264.mp4") else { return };
        set_hardware_decode(true);
        let result = FfmpegDecoder::open(path).and_then(|mut d| d.decode_video_frame(1_000_000));
        set_hardware_decode(false);
        let frame = result.expect("frame");
        assert_eq!((frame.width, frame.height), (1920, 1080));
    }

    #[test]
    fn keyframe_decode_lands_at_or_before_the_target() {
        let Some(path) = sample("sample_1080p_h264.mp4") else { return };
        let mut d = FfmpegDecoder::open(path).expect("open");
        let k = d.decode_keyframe(2_500_000).expect("frame");
        assert!(k.pts_us <= 2_500_000);
    }
}
