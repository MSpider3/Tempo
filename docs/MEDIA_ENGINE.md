# Media Engine — Tempo 2

> **Status (2026-10-03): design problems known, not yet rewritten.** The goals in §1 still
> stand. These parts need to change before they are implemented as written:
>
> - **§4.2 cache size**: 512 MB holds about 61 RGBA 1080p frames, not 170. Keep frames in
>   YUV (about 170 frames) and convert in the shader.
> - **§3.2 seeking**: frames decoded on the way to the target are thrown away. While
>   scrubbing, show keyframes; decode the exact frame when the pointer pauses.
> - **§3.1 one decoder per source**: two clips from the same file, or source and timeline
>   viewer together, will seek back and forth. Use a small pool of decoders, capped in number.
>   Audio and video need separate demuxers.
> - **§8.2 proxies**: same resolution as the source is not a proxy. Use 540p or 720p with a
>   short keyframe interval. Start one only when playback really drops frames, and pause
>   proxy encoding during playback.
> - **§5.4 display path**: hand hardware-decoded frames to GTK as dmabuf textures instead of
>   downloading them to CPU memory.
> - **§7 thumbnails and §6.2 waveforms**: cache both on disk under `~/.cache/tempo/`, not in
>   the project file and not in the frame cache.
> - **Variable-frame-rate sources** are not covered and must be.
> - **§9.1 ring buffer** of 4096 samples is about 85 ms; use about half a second.

**Version:** 1.0  
**Crate:** `tempo-media`  

This document covers every aspect of how Tempo 2 handles media files: import, probing, lazy
decoding, the frame cache, proxy generation, hardware acceleration, and format support. This
is the most performance-critical part of the application.

---

## 1. Design Philosophy

The core rule: **a file imported into Tempo 2 is usable immediately, with zero pre-processing.**

Most Linux video editors (Kdenlive, OpenShot, Shotcut) require the user to either convert files
to the project format or wait for proxy generation before editing can begin. DaVinci Resolve does
not do this. Tempo 2 follows DaVinci Resolve's model:

1. **Import = probe only.** Open the container, read the stream index, close. Done in < 100ms.
2. **Decode on demand.** Frames are decoded only when the render thread requests them.
3. **Proxy is optional and background.** If a clip is too slow to decode in real time, a proxy
   is generated in the background. Editing continues immediately using original frames at lower
   quality, then silently switches to proxy when ready.
4. **Export always uses originals.** Proxy files are never used for export.

---

## 2. Media Import Flow

```
User drags file(s) onto Media Pool
        │
        ▼
[UI Thread] MediaEngine::probe(path) dispatched to Decode Thread Pool
        │
        ▼
[Decode Pool] FFmpeg: avformat_open_input() → avformat_find_stream_info()
  Read: container format, stream count, video/audio codec info,
        duration, creation_time, rotate metadata
  Seek: to keyframe at 10% of duration → decode single frame → extract thumbnail
  Close: avformat_close_input()
        │
        ▼
[Decode Pool → UI Thread] glib::MainContext::invoke
  MediaSource struct ready → insert into MediaPool
  Thumbnail available → update Media Pool grid cell
        │
        ▼
[Background Thread] Generate filmstrip thumbnails (8 evenly-spaced frames)
  Each thumbnail decoded and cached as 160×90 RGBA
        │
        ▼
[Background Thread] Generate audio waveform (if source has audio)
  See §6 for waveform generation algorithm
```

### 2.1 Probe Data Extracted

```
For each file:
  container_format    (e.g., "mov,mp4,m4a,3gp,3g2,mj2")
  duration_us         (microseconds)
  bit_rate            (bits/sec, overall)

For video streams (first video stream used if multiple):
  codec_name          (e.g., "h264", "hevc", "vp9", "av1")
  width, height
  fps_num, fps_den    (rational, e.g., 30000/1001 for 29.97)
  color_range         (MPEG=limited, JPEG=full)
  color_space         (BT.601, BT.709, BT.2020)
  bit_depth           (8, 10, 12)
  has_alpha           (based on pixel format)

For audio streams (first audio stream):
  codec_name          (e.g., "aac", "mp3", "flac")
  sample_rate
  channels
  channel_layout
  bit_rate

Rotation metadata (from container tags → used to pre-rotate clips on display)
```

---

## 3. Lazy Frame Decoder

### 3.1 Architecture

Each `MediaSource` has an associated `FfmpegDecoder` that is lazy-initialized on first frame
request. The decoder is shared across the decode thread pool via `Arc<Mutex<FfmpegDecoder>>`.

```rust
pub struct FfmpegDecoder {
    format_ctx: *mut ffmpeg::AVFormatContext,   // opened container
    video_stream_idx: i32,
    audio_stream_idx: i32,
    video_codec_ctx: *mut ffmpeg::AVCodecContext,
    audio_codec_ctx: *mut ffmpeg::AVCodecContext,
    vaapi_device_ctx: Option<*mut ffmpeg::AVBufferRef>, // None = no VAAPI

    // Seek optimization: track last decoded PTS to avoid backward seeks when possible
    last_decoded_pts_us: TimeUs,
    last_keyframe_pts_us: TimeUs,
}
```

**Safety note:** All FFmpeg AVContext pointers are owned by `FfmpegDecoder` and freed in `Drop`.
FFmpeg is not thread-safe per context, so the `Mutex` ensures single-threaded access per source.
Multiple sources decode in parallel (different Mutexes).

### 3.2 Seek Algorithm

When a frame at `target_pts_us` is requested:

```
1. If target_pts_us is within [last_decoded_pts_us, last_decoded_pts_us + 2 frame durations]:
   → Continue reading forward from current position (cheapest path, no seek)

2. Else if target_pts_us > last_decoded_pts_us and target_pts_us < last_decoded_pts_us + 5s:
   → Read forward until target (cheap for small forward jumps)

3. Else:
   → av_seek_frame(format_ctx, video_stream_idx, target_pts_us, AVSEEK_FLAG_BACKWARD)
     This seeks to the nearest keyframe at or before target_pts_us
   → Decode forward from keyframe until we reach target_pts_us
   → Discard intermediate frames (they are not cached — only the requested frame is)

Note: "Decode forward" means calling avcodec_receive_frame() in a loop until frame.pts >= target
```

### 3.3 Frame Request Flow

```rust
// Called from Decode Thread Pool
pub fn decode_video_frame(
    &self,
    source_id: SourceId,
    source_offset_us: TimeUs,
) -> Result<VideoFrame> {
    let handle = self.sources.get(&source_id).ok_or(Error::SourceNotFound)?;

    // 1. Check frame cache — may already be there from earlier scrub
    let cache_key = FrameCacheKey { source_id, pts_us: quantize(source_offset_us) };
    if let Some(texture) = self.frame_cache.read().get(&cache_key) {
        return Ok(VideoFrame::from_cached_texture(texture));
    }

    // 2. Lock decoder and seek/decode
    let mut decoder = handle.decoder.lock();
    let avframe = decoder.decode_at(source_offset_us)?;

    // 3. Convert pixel format if needed (YUV→RGB for wgpu if no hardware path)
    let rgb_frame = convert_to_rgb(&avframe)?;

    // 4. Insert into frame cache (uploads to GPU)
    let video_frame = VideoFrame::from_avframe(rgb_frame);
    self.frame_cache.write().insert(cache_key, video_frame.clone(), &self.device);

    Ok(video_frame)
}
```

### 3.4 Pre-fetch Strategy

The render thread, when entering play mode, spawns a pre-fetch task on the decode thread pool.
This task decodes the next N frames ahead of the playhead and inserts them into the cache.
N = `playback.preroll_frames` (default 10). For 30fps, this is 333ms of look-ahead.

Pre-fetch is cancelled immediately when:
- Playback stops
- A seek occurs
- The source clip ends

---

## 4. Frame Cache

### 4.1 LRU Implementation

```rust
// Uses indexmap::IndexMap for ordered iteration + O(1) random access
// (equivalent to Python's OrderedDict)
pub struct FrameCache {
    entries: IndexMap<FrameCacheKey, CachedFrame>,
    current_bytes: usize,
    max_bytes: usize,
}

pub struct CachedFrame {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,   // pre-created for compositor use
    pub size_bytes: usize,
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
}

impl FrameCache {
    pub fn get(&mut self, key: &FrameCacheKey) -> Option<&CachedFrame> {
        // Move to back (most recently used) on hit
        if self.entries.contains_key(key) {
            self.entries.move_index(self.entries.get_index_of(key).unwrap(),
                                   self.entries.len() - 1);
            self.entries.get(key)
        } else {
            None
        }
    }

    pub fn insert(&mut self, key: FrameCacheKey, frame: VideoFrame, device: &wgpu::Device) {
        let texture = upload_frame_to_gpu(&frame, device);
        let size = frame.width as usize * frame.height as usize * 4; // RGBA bytes

        // Evict LRU entries until under budget
        while self.current_bytes + size > self.max_bytes && !self.entries.is_empty() {
            let (_, evicted) = self.entries.shift_remove_index(0); // front = LRU
            self.current_bytes -= evicted.size_bytes;
            // texture is dropped here → GPU memory freed
        }

        self.entries.insert(key, CachedFrame { texture, size_bytes: size, ... });
        self.current_bytes += size;
    }
}
```

### 4.2 Cache Sizing

Frame cache memory budget is user-configurable. Defaults and recommendations:

| RAM | Recommended Cache | Approx frames cached (1080p) |
|---|---|---|
| 8 GB | 512 MB | ~170 frames |
| 16 GB | 1024 MB | ~340 frames |
| 32 GB | 2048 MB | ~680 frames |

At 30fps, 170 cached frames = ~5.6 seconds of cached footage. This comfortably covers scrubbing
within a scene without re-decoding.

### 4.3 Cache Key Quantization

Cache keys quantize the source offset to the nearest frame boundary to avoid cache misses from
floating-point seek imprecision:

```rust
fn quantize(pts_us: TimeUs, fps_num: u32, fps_den: u32) -> TimeUs {
    let frame = us_to_frame(pts_us, fps_num, fps_den);
    frame_to_us(frame, fps_num, fps_den)
}
```

---

## 5. Hardware Acceleration (VAAPI)

### 5.1 Overview

VAAPI (Video Acceleration API) is the Linux standard for GPU-accelerated video decode/encode.
Intel iGPUs (supported from Ivy Bridge, 2012+) support H.264 decode. Broadwell+ supports HEVC.
AMD and newer Intel support AV1.

Enabling VAAPI reduces CPU usage for H.264 decode from ~30-60% to ~5-10% on a Core i3.
This is the primary reason 4K H.264 footage can play on low-end hardware.

### 5.2 VAAPI Initialization

```rust
pub fn init_vaapi() -> Option<VaapiContext> {
    // Find the DRM render node (typically /dev/dri/renderD128)
    let drm_node = find_drm_render_node()?;

    // Create FFmpeg hardware device context
    let mut hw_device_ctx: *mut ffmpeg::AVBufferRef = std::ptr::null_mut();
    let ret = unsafe {
        ffmpeg::av_hwdevice_ctx_create(
            &mut hw_device_ctx,
            ffmpeg::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
            drm_node.to_str()?,
            std::ptr::null_mut(),
            0,
        )
    };

    if ret < 0 {
        tracing::warn!("VAAPI unavailable: {}", ffmpeg_error_string(ret));
        return None;
    }

    tracing::info!("VAAPI initialized on {}", drm_node.display());
    Some(VaapiContext { hw_device_ctx })
}
```

### 5.3 VAAPI Decoder Setup

When VAAPI is available and the codec supports hardware decode:

```rust
fn setup_vaapi_decoder(
    codec_ctx: *mut ffmpeg::AVCodecContext,
    vaapi_ctx: &VaapiContext,
) -> Result<()> {
    // Attach hardware device context
    unsafe {
        (*codec_ctx).hw_device_ctx = ffmpeg::av_buffer_ref(vaapi_ctx.hw_device_ctx);
        (*codec_ctx).get_format = Some(vaapi_get_format_callback);
    }
    Ok(())
}

// Callback: select VAAPI pixel format when available
extern "C" fn vaapi_get_format_callback(
    _ctx: *mut ffmpeg::AVCodecContext,
    formats: *const ffmpeg::AVPixelFormat,
) -> ffmpeg::AVPixelFormat {
    // Walk the list, prefer AV_PIX_FMT_VAAPI
    let mut fmt = formats;
    while unsafe { *fmt } != ffmpeg::AVPixelFormat::AV_PIX_FMT_NONE {
        if unsafe { *fmt } == ffmpeg::AVPixelFormat::AV_PIX_FMT_VAAPI {
            return ffmpeg::AVPixelFormat::AV_PIX_FMT_VAAPI;
        }
        fmt = unsafe { fmt.add(1) };
    }
    unsafe { *formats } // fallback to first available
}
```

### 5.4 VAAPI Frame Download

VAAPI frames are stored in GPU memory. To use in wgpu compositor, either:
1. **DRM export** (zero-copy): Export VAAPI surface as DRM PRIME fd → import into wgpu as
   external texture. Complex but zero-copy. Implement in Phase 2.
2. **CPU download** (simple): `av_hwframe_transfer_data()` → CPU buffer → upload to wgpu.
   Simpler, used in Phase 1. Still much faster than software decode.

### 5.5 Codec Hardware Support Table

| Codec | VAAPI | Software Fallback |
|---|---|---|
| H.264 (AVC) | ✅ (Intel Ivy Bridge+) | ✅ libavcodec |
| H.265 (HEVC) | ✅ (Intel Broadwell+) | ✅ libavcodec |
| VP9 | ✅ (Intel Kaby Lake+) | ✅ libvpx |
| AV1 | ✅ (Intel Tiger Lake+) | ✅ libaom (slow) |
| VP8 | ❌ | ✅ libvpx |
| ProRes | ❌ | ✅ libavcodec |
| DNxHD | ❌ | ✅ libavcodec |
| MPEG-2 | ✅ (most) | ✅ libavcodec |
| MPEG-4 (xvid) | ❌ | ✅ libavcodec |

---

## 6. Waveform Generation

Waveforms are generated once per source file, cached to disk alongside the proxy.

### 6.1 Algorithm

```rust
pub fn generate_waveform(
    source_id: SourceId,
    source: &MediaSource,
    blocks: usize,        // number of visual blocks (one per pixel column in typical use)
) -> Result<WaveformData> {
    // Total samples in source
    let total_samples = (source.duration_us as f64 / 1_000_000.0
        * source.audio.as_ref()?.sample_rate as f64) as usize;
    let samples_per_block = total_samples / blocks;

    let mut decoder = FfmpegDecoder::open(&source.path)?;
    let mut blocks_out = Vec::with_capacity(blocks);
    let mut block_samples: Vec<f32> = Vec::with_capacity(samples_per_block);

    loop {
        match decoder.decode_audio_frame()? {
            Some(frame) => {
                // Convert all channels to mono by averaging
                for sample_idx in 0..frame.nb_samples {
                    let mut sum = 0f32;
                    for ch in 0..frame.channels {
                        sum += frame.sample(ch, sample_idx);
                    }
                    block_samples.push(sum / frame.channels as f32);

                    if block_samples.len() >= samples_per_block {
                        // Compute peak and RMS for this block
                        let peak_pos = block_samples.iter().cloned().fold(0f32, f32::max);
                        let peak_neg = block_samples.iter().cloned().fold(0f32, f32::min);
                        let rms = (block_samples.iter().map(|s| s * s).sum::<f32>()
                            / block_samples.len() as f32).sqrt();
                        blocks_out.push(WaveformBlock { peak_pos, peak_neg, rms });
                        block_samples.clear();
                    }
                }
            }
            None => break,
        }
    }

    Ok(WaveformData { blocks: blocks_out, sample_rate: ..., channels: ... })
}
```

### 6.2 Waveform Cache

Waveform data is stored at: `{proxy_dir}/{source_id}.waveform` as a flat binary file
(little-endian f32 triplets: peak_pos, peak_neg, rms per block).

If the cache file exists and the source file has not been modified (check mtime), it is
loaded instead of regenerating. Generation happens on the Decode Thread Pool at low priority,
same as thumbnail generation.

---

## 7. Thumbnail Generation

### 7.1 Media Pool Thumbnail (single frame, at 10% of clip)

- Resolution: 160 × 90 px, RGBA
- Generated during import probe
- Stored in memory only (not disk-cached)
- Re-generated on next launch (probe happens again)
- Future optimization: disk-cache in `{proxy_dir}/{source_id}.thumb`

### 7.2 Filmstrip (8 evenly-spaced frames along clip duration)

Generated after import completes, on Decode Thread Pool at low priority.
- Resolution: 160 × 90 px each, RGBA
- Stored in the `FrameCache` (not disk-cached — cheap to regenerate)
- As thumbnails trickle in, each is uploaded to a wgpu texture and the UI is notified to redraw
  the clip filmstrip

---

## 8. Proxy Generation

### 8.1 When to Generate Proxy

The proxy engine measures decode throughput on first clip play:

```rust
pub async fn should_proxy(source: &MediaSource, timeline_fps: f64) -> bool {
    let start = Instant::now();
    let _frame = decode_video_frame(source, source.duration_us / 2)?;  // mid-clip frame
    let elapsed = start.elapsed();

    // Decode time per frame must be < 1/fps seconds to sustain real-time playback
    let budget = Duration::from_secs_f64(1.0 / timeline_fps);

    // Add 20% margin — decoder throughput is not perfectly consistent
    elapsed > budget * 8 / 10  // true = proxy needed
}
```

If `should_proxy` returns true, proxy generation starts immediately in the background.

### 8.2 Proxy Encode Settings

```
Codec:       H.264 (libx264)
CRF:         18 (visually near-lossless, small file)
Resolution:  Same as original (preserve quality, just change codec)
FPS:         Same as original
Audio:       AAC 192 kbps (re-encoded for AAC fast decode)
Preset:      veryfast (balance encode speed vs size)
Threads:     All proxy pool threads (typically 1-2, to avoid starving decode pool)
Output:      {proxy_dir}/{source_id}.proxy.mp4
```

### 8.3 Proxy Progress Reporting

```rust
// Proxy encode loop (runs on proxy thread pool)
fn encode_proxy(source: &MediaSource, output: &Path, progress: Arc<AtomicF32>, cancel: Arc<AtomicBool>) {
    let mut encoder = FfmpegEncoder::new(output, PROXY_SETTINGS)?;
    let total_frames = (source.duration_us as f64 / 1e6 * fps).round() as u64;
    let mut frames_done = 0u64;

    loop {
        if cancel.load(Ordering::Relaxed) { break; }

        match decoder.decode_video_frame()? {
            Some(frame) => {
                encoder.encode_frame(frame)?;
                frames_done += 1;
                progress.store(frames_done as f32 / total_frames as f32, Ordering::Relaxed);

                // Report to UI every 30 frames
                if frames_done % 30 == 0 {
                    glib::MainContext::default().invoke(move || {
                        app_state.emit(AppMessage::ProxyProgress(source.id, progress_val));
                    });
                }
            }
            None => break,
        }
    }

    encoder.flush()?;
    glib::MainContext::default().invoke(|| {
        app_state.emit(AppMessage::ProxyComplete(source.id, output.to_path_buf()));
    });
}
```

### 8.4 Switching to Proxy

When `AppMessage::ProxyComplete` is received by the UI thread:
1. Update `MediaSource.proxy_path` in the media pool
2. On next frame request for this source, `FfmpegDecoder` opens the proxy file instead
3. No UI indication needed — playback smoothes out naturally
4. "PROXY" badge appears in viewer top-right to inform the user

---

## 9. Audio Decode and Mix

### 9.1 Audio Decode Thread

Audio decoding does NOT happen on the audio thread (which is real-time). Instead:

1. **Decode thread pool** decodes audio frames up to `preroll_frames` ahead of the playhead
2. Decoded PCM frames go into a per-clip `AudioRingBuffer` (lock-free ring buffer)
3. **Audio thread** reads from the ring buffers, mixes, and writes to PipeWire

```rust
pub struct AudioRingBuffer {
    buf: Box<[f32]>,          // size: 4096 * channels (interleaved f32)
    write_idx: AtomicUsize,
    read_idx: AtomicUsize,
}
```

### 9.2 Mix Algorithm

For each PipeWire process callback (frame size N samples at sample rate R):

```
For each active audio clip at current playhead position:
    1. Read N samples from clip's AudioRingBuffer
    2. Apply clip volume:   sample *= clip.properties.volume
    3. Apply clip pan:      left *= (1.0 - max(0, pan)); right *= (1.0 + min(0, pan))
    4. Apply track mute:    if track.muted: sample = 0.0
    5. Accumulate into mix buffer

Apply master volume to mix buffer
Clamp mix buffer to [-1.0, 1.0]
Write mix buffer to PipeWire stream
```

### 9.3 Audio/Video Sync

A/V sync is maintained via the shared `PlaybackState.position_us` atomic. Both the audio thread
and the render thread read the same source of truth. The audio thread is the sync master:
it updates `position_us` on each callback, the render thread reads it to determine which frame
to composite. If the render thread falls behind (dropped frames), audio continues uninterrupted.

---

## 10. Supported Formats Summary

### 10.1 Video Formats (Input)

| Container | Video Codecs | Audio Codecs | Notes |
|---|---|---|---|
| MP4 / M4V | H.264, H.265, MPEG-4, AV1 | AAC, MP3, AC-3 | Most common |
| MOV | H.264, H.265, ProRes, DNxHD | AAC, PCM | Apple; ProRes for pro cameras |
| MKV | H.264, H.265, VP8, VP9, AV1 | AAC, MP3, FLAC, Opus | Open container |
| AVI | H.264, MPEG-4, DivX | MP3, PCM | Legacy |
| WebM | VP8, VP9, AV1 | Vorbis, Opus | Web video |
| TS / M2TS | H.264, H.265, MPEG-2 | AAC, AC-3 | Broadcast, Blu-ray |
| MXF | DNxHD, ProRes, MPEG-2 | PCM, AAC | Professional acquisition |
| FLV | H.264 | AAC, MP3 | Legacy streaming |

### 10.2 Audio-Only Formats (Input)

MP3, AAC (M4A), FLAC, WAV, OGG Vorbis, Opus, AIFF, WMA (via FFmpeg)

### 10.3 Image Formats (Input)

JPEG, PNG, WebP, TIFF, BMP, GIF (first frame only), AVIF

### 10.4 Export Formats (Output)

| Container | Video Codec Options | Audio Codec Options |
|---|---|---|
| MP4 | H.264, H.265, AV1 | AAC, MP3, Opus |
| MOV | H.264, H.265, ProRes | AAC, PCM |
| MKV | H.264, H.265, VP9, AV1 | AAC, FLAC, Opus |
| WebM | VP9, AV1 | Opus, Vorbis |

---

## 11. File Discovery and Relink

### 11.1 Missing File Detection

On project open, for every `MediaSource` in the project:
```
1. Check if source.path exists → if yes, done
2. Check if source.relative_path (relative to project file) exists → if yes, update source.path
3. File is missing → mark source.is_missing = true
```

### 11.2 Relink Algorithm

When user clicks "Relink All" and picks a folder:
```
For each missing source:
    Search the chosen folder recursively for a file named source.path.filename()
    If found:
        Verify duration matches within 1 second (sanity check)
        Update source.path to found path
        Mark source.is_missing = false
    If multiple candidates found:
        Show per-file selection dialog
    If not found:
        Leave as missing (shown with red border in Media Pool)
```
