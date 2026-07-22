"""Media import pipeline for Tempo.

Handles file validation, media type detection, metadata probing, and constructing
MediaItem models. Does not perform proxy/thumbnail generation.
"""

import contextlib
import uuid
from pathlib import Path
from typing import Literal

from tempo.core.models import MediaItem
from tempo.utils.cache import CacheManager
from tempo.utils.ffmpeg import probe


class MediaImportError(Exception):
    """Raised when media import process fails."""


class UnsupportedFormatError(MediaImportError):
    """Raised when file format is not supported by Tempo."""


SUPPORTED_VIDEO_EXTENSIONS = frozenset(["mp4", "mov", "mkv", "avi", "webm"])
SUPPORTED_AUDIO_EXTENSIONS = frozenset(["mp3", "wav", "aac", "flac", "ogg"])
SUPPORTED_IMAGE_EXTENSIONS = frozenset(["jpg", "jpeg", "png", "webp"])


def detect_media_type(path: Path) -> Literal["video", "audio", "image"] | None:
    """Detect the media type of a file based on its extension.

    Args:
        path: Path to the media file.

    Returns:
        One of "video", "audio", "image", or None if extension is not supported.
    """
    ext = path.suffix.lower().lstrip(".")
    if ext in SUPPORTED_VIDEO_EXTENSIONS:
        return "video"
    elif ext in SUPPORTED_AUDIO_EXTENSIONS:
        return "audio"
    elif ext in SUPPORTED_IMAGE_EXTENSIONS:
        return "image"
    else:
        return None


def import_media(path: Path, cache: CacheManager) -> MediaItem:
    """Validate, probe, and import a media file into a MediaItem.

    Does not trigger background tasks (proxies/thumbnails).

    Args:
        path: The path to the source media file.
        cache: CacheManager instance.

    Returns:
        A fully constructed and populated MediaItem.

    Raises:
        FileNotFoundError: If the media file does not exist.
        UnsupportedFormatError: If format is not supported.
        MediaImportError: If ffprobe/metadata extraction fails.
    """
    if not path.exists():
        raise FileNotFoundError(f"Media file not found: {path}")

    media_type = detect_media_type(path)
    if not media_type:
        raise UnsupportedFormatError(f"Unsupported file format: {path.suffix}")

    try:
        info = probe(path)
    except Exception as e:
        raise MediaImportError(f"Failed to probe media file: {e}") from e

    streams = info.get("streams", [])
    video_stream = next((s for s in streams if s.get("codec_type") == "video"), None)
    audio_stream = next((s for s in streams if s.get("codec_type") == "audio"), None)

    # Validate stream existence based on type
    if media_type == "video" and not video_stream:
        raise UnsupportedFormatError(f"Video file has no video stream: {path}")
    if media_type == "audio" and not audio_stream:
        raise UnsupportedFormatError(f"Audio file has no audio stream: {path}")
    if media_type == "image" and not video_stream:
        raise UnsupportedFormatError(f"Image file has no valid image stream: {path}")

    # Extract duration
    duration = 0.0
    if media_type in ("video", "audio"):
        format_info = info.get("format", {})
        duration_str = format_info.get("duration")
        if duration_str is not None:
            with contextlib.suppress(ValueError):
                duration = float(duration_str)
        if duration <= 0.0:
            # Fallback to stream duration
            stream = video_stream if media_type == "video" else audio_stream
            if stream:
                stream_duration_str = stream.get("duration")
                if stream_duration_str is not None:
                    with contextlib.suppress(ValueError):
                        duration = float(stream_duration_str)

    # Extract dimensions & frame rate
    width = None
    height = None
    fps = None

    if video_stream:
        try:
            width = int(video_stream["width"])
            height = int(video_stream["height"])
        except (KeyError, ValueError):
            pass

        # Frame rate parsing
        r_fps = video_stream.get("r_frame_rate", "")
        fps_parts = r_fps.split("/")
        if len(fps_parts) == 2:
            try:
                num = float(fps_parts[0])
                den = float(fps_parts[1])
                if den > 0:
                    fps = num / den
            except ValueError:
                pass

    has_audio = audio_stream is not None

    return MediaItem(
        id=str(uuid.uuid4()),
        original_path=str(path.resolve()),
        proxy_path=None,
        type=media_type,
        duration=duration,
        width=width,
        height=height,
        fps=fps,
        has_audio=has_audio,
        thumbnail_path=None,
    )
