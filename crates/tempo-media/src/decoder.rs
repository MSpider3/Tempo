use std::path::{Path, PathBuf};
use ffmpeg_next::format::{self, Pixel};
use ffmpeg_next::media::Type as FfmpegMediaType;
use ffmpeg_next::software::scaling::{context::Context as ScalerContext, flag::Flags};
use ffmpeg_next::software::resampling::context::Context as ResamplerContext;
use ffmpeg_next::{ChannelLayout, Rational};

use crate::error::{MediaError, Result};
use crate::probe::ensure_ffmpeg_init;

pub const SEEK_THRESHOLD_US: i64 = 2_000_000; // 2.0 seconds

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba,
    Yuv420p,
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
    pub pts_us: i64,
}

#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: u8,
    pub samples: Vec<f32>,
    pub pts_us: i64,
}

pub struct FfmpegDecoder {
    pub path: PathBuf,
    ictx: format::context::Input,
    video_stream_index: Option<usize>,
    video_decoder: Option<ffmpeg_next::decoder::Video>,
    video_time_base: Rational,
    video_width: u32,
    video_height: u32,
    out_width: u32,
    out_height: u32,
    src_format: Pixel,
    scaler: Option<ScalerContext>,
    audio_stream_index: Option<usize>,
    audio_decoder: Option<ffmpeg_next::decoder::Audio>,
    pub audio_time_base: Rational,
    pub audio_sample_rate: u32,
    pub audio_channels: u16,
    resampler: Option<ResamplerContext>,
    last_decoded_pts_us: i64,
    last_audio_pts_us: i64,
    /// The request and result of the last exact decode. Asking again for a time
    /// that falls on the same frame returns this copy instead of reading on.
    last_exact: Option<(i64, VideoFrame)>,
}

// FFmpeg decoding and scaling contexts are safe to transfer across threads when synchronized (e.g. behind a Mutex).
unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        ensure_ffmpeg_init();
        let path = path.as_ref().to_path_buf();

        if !path.exists() {
            return Err(MediaError::FileNotFound(path.to_string_lossy().to_string()));
        }

        let ictx = format::input(&path).map_err(MediaError::Ffmpeg)?;

        // Video setup
        let mut video_stream_index = None;
        let mut video_decoder = None;
        let mut video_time_base = Rational::new(1, 1_000_000);
        let mut video_width = 0;
        let mut video_height = 0;
        let mut scaler = None;
        let mut src_pix = Pixel::None;

        if let Some(video_stream) = ictx.streams().best(FfmpegMediaType::Video) {
            let idx = video_stream.index();
            video_time_base = video_stream.time_base();
            let mut codec_ctx = ffmpeg_next::codec::context::Context::from_parameters(
                video_stream.parameters(),
            ).map_err(MediaError::Ffmpeg)?;
            codec_ctx.set_threading(ffmpeg_next::codec::threading::Config {
                count: 0,
                // Frame threading: most H.264/HEVC files have one slice per frame,
                // so slice threading would decode on a single core.
                kind: ffmpeg_next::codec::threading::Type::Frame,
            });

            if let Ok(dec) = codec_ctx.decoder().video() {
                video_width = dec.width();
                video_height = dec.height();
                let src_format = dec.format();
                src_pix = src_format;

                if let Ok(sc) = ScalerContext::get(
                    src_format,
                    video_width,
                    video_height,
                    Pixel::RGBA,
                    video_width,
                    video_height,
                    Flags::BILINEAR,
                ) {
                    scaler = Some(sc);
                }

                video_stream_index = Some(idx);
                video_decoder = Some(dec);
            }
        }

        // Audio setup
        let mut audio_stream_index = None;
        let mut audio_decoder = None;
        let mut audio_time_base = Rational::new(1, 1_000_000);
        let mut audio_sample_rate = 48000;
        let mut audio_channels = 2;
        let mut resampler = None;

        if let Some(audio_stream) = ictx.streams().best(FfmpegMediaType::Audio) {
            let idx = audio_stream.index();
            audio_time_base = audio_stream.time_base();
            let codec_ctx = ffmpeg_next::codec::context::Context::from_parameters(
                audio_stream.parameters(),
            ).map_err(MediaError::Ffmpeg)?;

            if let Ok(dec) = codec_ctx.decoder().audio() {
                audio_sample_rate = dec.rate();
                audio_channels = dec.channels();

                let src_layout = dec.channel_layout();
                let src_format = dec.format();

                let res = ResamplerContext::get(
                    src_format,
                    src_layout,
                    dec.rate(),
                    ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
                    ChannelLayout::STEREO,
                    48000,
                );
                if let Ok(r) = res {
                    resampler = Some(r);
                }

                audio_stream_index = Some(idx);
                audio_decoder = Some(dec);
            }
        }

        Ok(Self {
            path,
            ictx,
            video_stream_index,
            video_decoder,
            video_time_base,
            video_width,
            video_height,
            out_width: video_width,
            out_height: video_height,
            src_format: src_pix,
            scaler,
            audio_stream_index,
            audio_decoder,
            audio_time_base,
            audio_sample_rate,
            audio_channels,
            resampler,
            last_decoded_pts_us: -1,
            last_audio_pts_us: -1,
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
        if (w, h) == (self.out_width, self.out_height) || self.video_decoder.is_none() {
            return;
        }
        if let Ok(sc) = ScalerContext::get(
            self.src_format,
            self.video_width,
            self.video_height,
            Pixel::RGBA,
            w,
            h,
            Flags::FAST_BILINEAR,
        ) {
            self.scaler = Some(sc);
            self.out_width = w;
            self.out_height = h;
            self.last_exact = None;
        }
    }

    fn scale_frame(
        &mut self,
        decoded_frame: &ffmpeg_next::frame::Video,
        pts_us: i64,
    ) -> Result<VideoFrame> {
        let scaler = self.scaler.as_mut().ok_or_else(|| {
            MediaError::StreamNotFound("Video scaler not initialized".to_string())
        })?;
        let mut rgba_frame = ffmpeg_next::frame::Video::empty();
        if scaler.run(decoded_frame, &mut rgba_frame).is_ok() {
            let width = self.out_width;
            let height = self.out_height;
            let data = rgba_frame.data(0);
            let stride = rgba_frame.stride(0) as usize;
            let row_bytes = (width * 4) as usize;
            let total = row_bytes * height as usize;
            let mut buffer = Vec::with_capacity(total);

            if stride == row_bytes && data.len() >= total {
                buffer.extend_from_slice(&data[..total]);
            } else {
                for y in 0..height as usize {
                    let start = y * stride;
                    let end = start + row_bytes;
                    if end <= data.len() {
                        buffer.extend_from_slice(&data[start..end]);
                    }
                }
            }

            Ok(VideoFrame {
                width,
                height,
                format: PixelFormat::Rgba,
                data: buffer,
                pts_us,
            })
        } else {
            Err(MediaError::FrameDecodeFailed)
        }
    }

    /// Exact frame at or after `target_pts_us`.
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
        let v_idx = self.video_stream_index.ok_or_else(|| {
            MediaError::StreamNotFound("No video stream found".to_string())
        })?;
        let v_decoder = self.video_decoder.as_mut().ok_or_else(|| {
            MediaError::StreamNotFound("Video decoder not initialized".to_string())
        })?;

        // 1. Check if seek is needed
        let need_seek = keyframe_only
            || self.last_decoded_pts_us < 0
            || target_pts_us < self.last_decoded_pts_us
            || target_pts_us - self.last_decoded_pts_us > SEEK_THRESHOLD_US;

        if need_seek {
            // Seek backwards to nearest keyframe
            let _ = self.ictx.seek(target_pts_us, ..target_pts_us);
            v_decoder.flush();
            self.last_decoded_pts_us = -1;
        }

        let mut decoded_frame = ffmpeg_next::frame::Video::empty();
        let mut last_decoded_raw: Option<(ffmpeg_next::frame::Video, i64)> = None;

        for (stream, packet) in self.ictx.packets() {
            if stream.index() == v_idx {
                if v_decoder.send_packet(&packet).is_ok() {
                    while v_decoder.receive_frame(&mut decoded_frame).is_ok() {
                        let pts = decoded_frame.pts().unwrap_or(0);
                        let pts_us = if self.video_time_base.denominator() > 0 {
                            (pts as i128 * self.video_time_base.numerator() as i128 * 1_000_000
                                / self.video_time_base.denominator() as i128) as i64
                        } else {
                            target_pts_us
                        };

                        self.last_decoded_pts_us = pts_us;

                        if keyframe_only || pts_us >= target_pts_us {
                            return self.scale_frame(&decoded_frame, pts_us);
                        } else {
                            last_decoded_raw = Some((decoded_frame.clone(), pts_us));
                        }
                    }
                }
            }
        }

        // End of file: drain the frames the decoder is still holding.
        if let Some(v_decoder) = self.video_decoder.as_mut() {
            let _ = v_decoder.send_eof();
            while v_decoder.receive_frame(&mut decoded_frame).is_ok() {
                let pts = decoded_frame.pts().unwrap_or(0);
                let pts_us = (pts as i128 * self.video_time_base.numerator() as i128 * 1_000_000
                    / self.video_time_base.denominator().max(1) as i128) as i64;
                if pts_us >= target_pts_us {
                    self.last_decoded_pts_us = -1; // decoder is drained; next call must seek
                    return self.scale_frame(&decoded_frame, pts_us);
                }
                last_decoded_raw = Some((decoded_frame.clone(), pts_us));
            }
            self.last_decoded_pts_us = -1;
        }

        if let Some((raw, pts)) = last_decoded_raw {
            self.scale_frame(&raw, pts)
        } else {
            Err(MediaError::FrameDecodeFailed)
        }
    }

    pub fn decode_audio_range(
        &mut self,
        start_us: i64,
        end_us: i64,
        target_sample_rate: u32,
        target_channels: u8,
    ) -> Result<AudioBuffer> {
        let duration_us = (end_us - start_us).max(0);
        let needed_samples = ((duration_us as i128 * target_sample_rate as i128
            * target_channels as i128)
            / 1_000_000) as usize;

        let a_idx = match self.audio_stream_index {
            Some(idx) => idx,
            None => {
                // Return silent buffer if no audio stream
                return Ok(AudioBuffer {
                    sample_rate: target_sample_rate,
                    channels: target_channels,
                    samples: vec![0.0f32; needed_samples],
                    pts_us: start_us,
                });
            }
        };

        let a_decoder = self.audio_decoder.as_mut().ok_or_else(|| {
            MediaError::StreamNotFound("Audio decoder not available".to_string())
        })?;

        // Seek if needed
        let need_seek = if self.last_audio_pts_us < 0 {
            true
        } else if start_us < self.last_audio_pts_us {
            true
        } else if start_us - self.last_audio_pts_us > SEEK_THRESHOLD_US {
            true
        } else {
            false
        };

        if need_seek {
            let _ = self.ictx.seek(start_us, ..start_us);
            a_decoder.flush();
            self.last_audio_pts_us = -1;
        }

        let mut samples_out: Vec<f32> = Vec::with_capacity(needed_samples);
        let mut decoded_audio = ffmpeg_next::frame::Audio::empty();
        let mut resampled_audio = ffmpeg_next::frame::Audio::empty();

        for (stream, packet) in self.ictx.packets() {
            if stream.index() == a_idx {
                if a_decoder.send_packet(&packet).is_ok() {
                    while a_decoder.receive_frame(&mut decoded_audio).is_ok() {
                        if let Some(resampler) = self.resampler.as_mut() {
                            if resampler.run(&decoded_audio, &mut resampled_audio).is_ok() {
                                let num_samples = resampled_audio.samples();
                                let ch = resampled_audio.channels() as usize;
                                let data = resampled_audio.data(0);
                                let f32_slice: &[f32] = unsafe {
                                    std::slice::from_raw_parts(
                                        data.as_ptr() as *const f32,
                                        num_samples * ch,
                                    )
                                };
                                samples_out.extend_from_slice(f32_slice);
                            }
                        }

                        if samples_out.len() >= needed_samples {
                            break;
                        }
                    }
                }
            }

            if samples_out.len() >= needed_samples {
                break;
            }
        }

        samples_out.resize(needed_samples, 0.0f32);
        self.last_audio_pts_us = end_us;

        Ok(AudioBuffer {
            sample_rate: target_sample_rate,
            channels: target_channels,
            samples: samples_out,
            pts_us: start_us,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hwaccel::VaapiHwContext;

    #[test]
    fn test_decoder_open_and_decode_video() {
        let path = Path::new("tests/media/sample_1080p_h264.mp4");
        if !path.exists() {
            return;
        }

        let mut decoder = FfmpegDecoder::open(path).expect("Failed to open decoder");
        assert_eq!(decoder.video_width(), 1920);
        assert_eq!(decoder.video_height(), 1080);

        let frame = decoder.decode_video_frame(0).expect("Failed to decode frame 0");
        assert_eq!(frame.width, 1920);
        assert_eq!(frame.height, 1080);
        assert_eq!(frame.format, PixelFormat::Rgba);
        assert_eq!(frame.data.len(), 1920 * 1080 * 4);
    }

    #[test]
    fn test_decoder_seek_and_decode() {
        let path = Path::new("tests/media/sample_1080p_h264.mp4");
        if !path.exists() {
            return;
        }

        let mut decoder = FfmpegDecoder::open(path).expect("Failed to open decoder");
        // Seek to 1.5 seconds (1_500_000 us)
        let frame = decoder.decode_video_frame(1_500_000).expect("Failed to seek & decode");
        assert!(frame.pts_us >= 1_000_000);
        assert_eq!(frame.data.len(), 1920 * 1080 * 4);
    }

    #[test]
    fn test_decoder_audio_range() {
        let path = Path::new("tests/media/sample_1080p_h264.mp4");
        if !path.exists() {
            return;
        }

        let mut decoder = FfmpegDecoder::open(path).expect("Failed to open decoder");
        let audio = decoder.decode_audio_range(0, 100_000, 48000, 2).expect("Failed to decode audio");
        assert_eq!(audio.sample_rate, 48000);
        assert_eq!(audio.channels, 2);
        // 100ms at 48000 Hz stereo = 4800 * 2 = 9600 samples
        assert_eq!(audio.samples.len(), 9600);
    }

    #[test]
    fn test_vaapi_hw_context() {
        let ctx = VaapiHwContext::new();
        // Should succeed or return None without panicking
        if let Some(c) = ctx {
            assert!(c.is_available);
        }
    }
}
