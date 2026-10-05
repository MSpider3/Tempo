use std::path::{Path, PathBuf};
use std::sync::Once;
use ffmpeg_next::format::{self, Pixel};
use ffmpeg_next::media::Type as FfmpegMediaType;
use ffmpeg_next::software::scaling::{context::Context as ScalerContext, flag::Flags};
use tempo_timeline::types::{MediaSource, MediaType, RationalFps};

use crate::error::{MediaError, Result};

static FFMPEG_INIT: Once = Once::new();

pub fn ensure_ffmpeg_init() {
    FFMPEG_INIT.call_once(|| {
        if let Err(e) = ffmpeg_next::init() {
            tracing::error!("Failed to initialize ffmpeg: {:?}", e);
        }
    });
}

#[derive(Debug, Clone)]
pub struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub fps: RationalFps,
    pub codec: String,
    pub color_range: Option<u8>,
    pub color_space: Option<u8>,
    pub bit_depth: Option<u8>,
    pub has_alpha: bool,
}

#[derive(Debug, Clone)]
pub struct AudioMetadata {
    pub sample_rate: u32,
    pub channels: u32,
    pub codec: String,
    pub bit_rate: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct MediaMetadata {
    pub media_type: MediaType,
    pub duration_us: i64,
    pub video: Option<VideoMetadata>,
    pub audio: Option<AudioMetadata>,
    pub thumbnail_rgba: Option<Vec<u8>>,
}

pub struct MediaEngine;

impl MediaEngine {
    pub fn probe(path: &Path) -> Result<MediaMetadata> {
        ensure_ffmpeg_init();

        if !path.exists() {
            return Err(MediaError::FileNotFound(path.to_string_lossy().to_string()));
        }

        let mut ictx = format::input(path).map_err(MediaError::Ffmpeg)?;

        let duration_us = if ictx.duration() > 0 {
            // ffmpeg duration is in AV_TIME_BASE units (1,000,000 us)
            ictx.duration()
        } else {
            0
        };

        // Check for video stream
        let mut video_meta: Option<VideoMetadata> = None;
        let mut thumbnail_rgba: Option<Vec<u8>> = None;

        if let Some(video_stream) = ictx.streams().best(FfmpegMediaType::Video) {
            let video_stream_index = video_stream.index();
            let avg_fps = video_stream.avg_frame_rate();
            let fps = if avg_fps.numerator() > 0 && avg_fps.denominator() > 0 {
                RationalFps {
                    num: avg_fps.numerator() as u32,
                    den: avg_fps.denominator() as u32,
                }
            } else {
                RationalFps::FPS_30
            };

            let codec_ctx = ffmpeg_next::codec::context::Context::from_parameters(
                video_stream.parameters(),
            ).map_err(MediaError::Ffmpeg)?;

            if let Ok(video_decoder) = codec_ctx.decoder().video() {
                let width = video_decoder.width();
                let height = video_decoder.height();
                let codec_name = video_decoder.codec().map(|c| c.name().to_string()).unwrap_or_default();
                let format = video_decoder.format();

                let has_alpha = matches!(
                    format,
                    Pixel::RGBA | Pixel::BGRA | Pixel::ARGB | Pixel::ABGR | Pixel::YUVA420P | Pixel::YUVA444P
                );

                video_meta = Some(VideoMetadata {
                    width,
                    height,
                    fps,
                    codec: codec_name,
                    color_range: Some(video_decoder.color_range() as u8),
                    color_space: Some(video_decoder.color_space() as u8),
                    bit_depth: Some(8),
                    has_alpha,
                });

                // Generate 160x90 thumbnail
                thumbnail_rgba = Self::extract_thumbnail(&mut ictx, video_stream_index, width, height, format);
            }
        }

        // Check for audio stream
        let mut audio_meta: Option<AudioMetadata> = None;
        if let Some(audio_stream) = ictx.streams().best(FfmpegMediaType::Audio) {
            let codec_ctx = ffmpeg_next::codec::context::Context::from_parameters(
                audio_stream.parameters(),
            ).map_err(MediaError::Ffmpeg)?;

            if let Ok(audio_decoder) = codec_ctx.decoder().audio() {
                let sample_rate = audio_decoder.rate();
                let channels = audio_decoder.channels() as u32;
                let codec_name = audio_decoder.codec().map(|c| c.name().to_string()).unwrap_or_default();
                let bit_rate = audio_decoder.bit_rate() as u32;

                audio_meta = Some(AudioMetadata {
                    sample_rate,
                    channels,
                    codec: codec_name,
                    bit_rate: if bit_rate > 0 { Some(bit_rate) } else { None },
                });
            }
        }

        let media_type = if video_meta.is_some() {
            // If duration is very short (1 frame) or image extension, consider image
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tiff") {
                MediaType::Image
            } else {
                MediaType::Video
            }
        } else if audio_meta.is_some() {
            MediaType::Audio
        } else {
            return Err(MediaError::NoStreamsFound(path.to_string_lossy().to_string()));
        };

        Ok(MediaMetadata {
            media_type,
            duration_us,
            video: video_meta,
            audio: audio_meta,
            thumbnail_rgba,
        })
    }

    pub fn create_media_source(path: &Path) -> Result<MediaSource> {
        let meta = Self::probe(path)?;
        let mut source = MediaSource::new(PathBuf::from(path), meta.media_type, meta.duration_us);

        if let Some(video) = meta.video {
            source.video_width = Some(video.width);
            source.video_height = Some(video.height);
            source.video_fps = Some(video.fps);
            source.video_codec = Some(video.codec);
            source.video_color_range = video.color_range;
            source.video_color_space = video.color_space;
            source.video_bit_depth = video.bit_depth;
            source.video_has_alpha = video.has_alpha;
        }

        if let Some(audio) = meta.audio {
            source.audio_sample_rate = Some(audio.sample_rate);
            source.audio_channels = Some(audio.channels);
            source.audio_codec = Some(audio.codec);
            source.audio_bit_rate = audio.bit_rate;
        }

        source.thumbnail_data = meta.thumbnail_rgba;
        Ok(source)
    }

    fn extract_thumbnail(
        ictx: &mut format::context::Input,
        stream_index: usize,
        src_width: u32,
        src_height: u32,
        src_format: Pixel,
    ) -> Option<Vec<u8>> {
        let stream = ictx.stream(stream_index)?;
        let codec_ctx = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters()).ok()?;
        let mut decoder = codec_ctx.decoder().video().ok()?;

        let mut scaler = ScalerContext::get(
            src_format,
            src_width,
            src_height,
            Pixel::RGBA,
            160,
            90,
            Flags::BILINEAR,
        ).ok()?;

        let mut decoded_frame = ffmpeg_next::frame::Video::empty();
        let mut thumb_frame = ffmpeg_next::frame::Video::empty();

        // Seek slightly into file if possible (e.g. 500ms) or first keyframe
        let _ = ictx.seek(500_000, ..);

        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_index
                && decoder.send_packet(&packet).is_ok()
                    && decoder.receive_frame(&mut decoded_frame).is_ok()
                        && scaler.run(&decoded_frame, &mut thumb_frame).is_ok() {
                            let data = thumb_frame.data(0);
                            let linesize = thumb_frame.stride(0);
                            let row_bytes = 160 * 4;
                            let mut rgba = Vec::with_capacity(160 * 90 * 4);

                            for y in 0..90 {
                                let start = y * linesize;
                                let end = start + row_bytes;
                                if end <= data.len() {
                                    rgba.extend_from_slice(&data[start..end]);
                                }
                            }

                            if rgba.len() == 160 * 90 * 4 {
                                return Some(rgba);
                            }
                        }
        }

        // Fallback: create solid dark thumbnail with 160x90 RGBA
        let mut fallback = vec![0u8; 160 * 90 * 4];
        for pixel in fallback.as_chunks_mut::<4>().0 {
            pixel[0] = 30;
            pixel[1] = 30;
            pixel[2] = 45;
            pixel[3] = 255;
        }
        Some(fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_missing_file() {
        let missing = Path::new("/nonexistent/video.mp4");
        let res = MediaEngine::probe(missing);
        assert!(matches!(res, Err(MediaError::FileNotFound(_))));
    }
}
