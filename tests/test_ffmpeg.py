"""Integration and unit tests for FFmpeg wrappers, cache, and media importer."""

from pathlib import Path

import pytest

from tempo.core.models import MediaItem
from tempo.media.importer import (
    detect_media_type,
    import_media,
)
from tempo.media.proxy import ProxyGenerator, target_proxy_height
from tempo.media.thumbnail import ThumbnailExtractor
from tempo.media.waveform import WaveformExtractor
from tempo.utils.cache import CacheManager
from tempo.utils.ffmpeg import (
    check_ffmpeg,
    extract_thumbnail,
    extract_waveform_peaks,
    generate_proxy,
    probe,
)

# Helper to verify FFmpeg/FFprobe availability
FFMPEG_AVAILABLE, FFPROBE_AVAILABLE = check_ffmpeg()
REQUIRES_FFMPEG = pytest.mark.skipif(
    not FFMPEG_AVAILABLE or not FFPROBE_AVAILABLE,
    reason="FFmpeg/FFprobe required for these tests",
)


def test_check_ffmpeg() -> None:
    """Test that check_ffmpeg returns booleans."""
    ffmpeg_ok, ffprobe_ok = check_ffmpeg()
    assert isinstance(ffmpeg_ok, bool)
    assert isinstance(ffprobe_ok, bool)


def test_detect_media_type() -> None:
    """Test media type detection for various extensions."""
    # Video
    assert detect_media_type(Path("clip.mp4")) == "video"
    assert detect_media_type(Path("movie.MOV")) == "video"
    assert detect_media_type(Path("raw.mkv")) == "video"

    # Audio
    assert detect_media_type(Path("track.mp3")) == "audio"
    assert detect_media_type(Path("sound.wav")) == "audio"

    # Image
    assert detect_media_type(Path("photo.jpg")) == "image"
    assert detect_media_type(Path("gfx.png")) == "image"

    # Unsupported
    assert detect_media_type(Path("doc.pdf")) is None
    assert detect_media_type(Path("archive.zip")) is None


def test_cache_manager_key_generation(tmp_path: Path) -> None:
    """Test CacheManager key generation is deterministic and correct."""
    cache = CacheManager(cache_root=tmp_path)
    test_file = tmp_path / "key_test.txt"
    test_file.write_text("Hello Cache", encoding="utf-8")

    key1 = cache.cache_key(test_file)
    key2 = cache.cache_key(test_file)

    assert key1 == key2
    assert len(key1) == 64  # SHA-256 is 64 hex chars


def test_cache_manager_paths(tmp_path: Path) -> None:
    """Test CacheManager path helper methods."""
    cache = CacheManager(cache_root=tmp_path)
    key = "dummykey123"

    assert cache.proxy_path(key) == tmp_path / "proxies" / f"{key}.mp4"
    assert cache.thumb_path(key) == tmp_path / "thumbs" / f"{key}.jpg"
    assert cache.waveform_path(key) == tmp_path / "waveforms" / f"{key}.json"


def test_target_proxy_height() -> None:
    """Test target proxy resolution mapping."""
    assert target_proxy_height(2160) == 360  # 4K
    assert target_proxy_height(1080) == 480  # 1080p
    assert target_proxy_height(720) is None  # 720p (no proxy needed)
    assert target_proxy_height(480) is None  # SD (no proxy needed)


@REQUIRES_FFMPEG
def test_probe_metadata(synthetic_video: Path) -> None:
    """Test probe returns valid parsed metadata for a real video file."""
    metadata = probe(synthetic_video)
    assert isinstance(metadata, dict)
    assert "streams" in metadata
    assert "format" in metadata

    streams = metadata["streams"]
    video_stream = next(s for s in streams if s["codec_type"] == "video")
    assert int(video_stream["width"]) == 320
    assert int(video_stream["height"]) == 240


@REQUIRES_FFMPEG
def test_extract_thumbnail(synthetic_video: Path, tmp_path: Path) -> None:
    """Test extract_thumbnail creates a thumbnail frame."""
    thumb_out = tmp_path / "thumb.jpg"
    extract_thumbnail(synthetic_video, thumb_out, 1.0)
    assert thumb_out.exists()
    assert thumb_out.stat().st_size > 0


@REQUIRES_FFMPEG
def test_generate_proxy(synthetic_video: Path, tmp_path: Path) -> None:
    """Test generate_proxy generates a valid scaled down proxy."""
    proxy_out = tmp_path / "proxy.mp4"
    # Target height 120 (extremely small for testing)
    generate_proxy(synthetic_video, proxy_out, 120)
    assert proxy_out.exists()

    # Verify proxy resolution
    meta = probe(proxy_out)
    video_stream = next(s for s in meta["streams"] if s["codec_type"] == "video")
    assert int(video_stream["height"]) == 120


@REQUIRES_FFMPEG
def test_extract_waveform_peaks(synthetic_video: Path) -> None:
    """Test waveform extraction generates normalized peak data."""
    peaks = extract_waveform_peaks(synthetic_video)
    assert isinstance(peaks, list)
    # 3 second video at 100 peak samples/sec should yield around 300 peaks
    assert len(peaks) > 0
    for p in peaks:
        assert isinstance(p, tuple)
        assert len(p) == 2
        assert -1.0 <= p[0] <= 1.0
        assert -1.0 <= p[1] <= 1.0


@REQUIRES_FFMPEG
def test_import_media_end_to_end(
    synthetic_video: Path, synthetic_image: Path, tmp_path: Path
) -> None:
    """Test full importer, proxy generation, and thumbnail extraction end-to-end."""
    cache = CacheManager(cache_root=tmp_path)

    # 1. Importer
    media_item = import_media(synthetic_video, cache)
    assert isinstance(media_item, MediaItem)
    assert media_item.type == "video"
    assert media_item.width == 320
    assert media_item.height == 240
    assert media_item.fps == 30.0
    assert media_item.has_audio
    assert media_item.duration == pytest.approx(3.0, abs=0.1)

    # 2. Proxy Generator (Force proxy generation by setting height as if it was 1080p)
    media_item.height = 1080
    proxy_gen = ProxyGenerator()
    proxy_path = proxy_gen.generate(media_item, cache)
    assert proxy_path.exists()
    assert proxy_path != Path(media_item.original_path)

    # 3. Thumbnail Extractor (Video)
    thumb_ext = ThumbnailExtractor()
    thumb_path = thumb_ext.extract(media_item, cache)
    assert thumb_path.exists()

    # 4. Thumbnail Extractor (Image)
    image_item = import_media(synthetic_image, cache)
    assert image_item.type == "image"
    image_thumb_path = thumb_ext.extract(image_item, cache)
    assert image_thumb_path.exists()

    # 5. Waveform Extractor
    wave_ext = WaveformExtractor(cache=cache)
    peaks = wave_ext.extract(media_item)
    assert len(peaks) > 0
    assert cache.waveform_exists(cache.cache_key(Path(media_item.original_path)))
