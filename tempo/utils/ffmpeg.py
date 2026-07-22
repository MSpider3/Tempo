"""FFmpeg and FFprobe wrapper for running subprocesses and parsing output.

Provides functions for metadata probing, proxy generation, thumbnail extraction,
and audio waveform peak extraction.
"""

import json
import shutil
import struct
import subprocess
from pathlib import Path
from typing import Any


class FFmpegError(Exception):
    """Raised when an FFmpeg execution fails."""


class FFprobeError(Exception):
    """Raised when an FFprobe execution fails."""


def check_ffmpeg() -> tuple[bool, bool]:
    """Check if ffmpeg and ffprobe command-line tools are available in the PATH.

    Returns:
        A tuple of (ffmpeg_available, ffprobe_available).
    """
    ffmpeg_ok = shutil.which("ffmpeg") is not None
    ffprobe_ok = shutil.which("ffprobe") is not None
    return ffmpeg_ok, ffprobe_ok


def probe(path: Path) -> dict[str, Any]:
    """Run ffprobe on the given file and return the parsed JSON metadata.

    Args:
        path: Path to the media file.

    Returns:
        The raw metadata dictionary parsed from ffprobe JSON output.

    Raises:
        FileNotFoundError: If ffprobe is not in PATH.
        FFprobeError: If ffprobe returns a non-zero exit code or times out.
    """
    if not shutil.which("ffprobe"):
        raise FileNotFoundError("ffprobe executable not found in PATH.")

    cmd = [
        "ffprobe",
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_streams",
        "-show_format",
        str(path.resolve()),
    ]

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=300,
            check=False,
        )
    except subprocess.TimeoutExpired as e:
        raise FFprobeError(f"ffprobe timed out on file {path}") from e
    except FileNotFoundError as e:
        raise FileNotFoundError("ffprobe executable not found in PATH.") from e

    if result.returncode != 0:
        raise FFprobeError(
            f"ffprobe failed with exit code {result.returncode} on file {path}.\n"
            f"Stderr: {result.stderr}"
        )

    try:
        metadata = json.loads(result.stdout)
        if not isinstance(metadata, dict):
            raise FFprobeError("ffprobe output is not a JSON object.")
        return metadata
    except json.JSONDecodeError as e:
        raise FFprobeError(f"Failed to parse ffprobe JSON output: {e}") from e


def generate_proxy(input_path: Path, output_path: Path, target_height: int) -> None:
    """Run FFmpeg to generate a low-resolution proxy of the input video.

    Args:
        input_path: Path to the source video file.
        output_path: Target path for the proxy file.
        target_height: Target vertical resolution (e.g. 480 or 360).

    Raises:
        FileNotFoundError: If ffmpeg is not in PATH.
        FFmpegError: If ffmpeg process fails or times out.
    """
    if not shutil.which("ffmpeg"):
        raise FileNotFoundError("ffmpeg executable not found in PATH.")

    # Ensure output parent directory exists
    output_path.parent.mkdir(parents=True, exist_ok=True)

    cmd = [
        "ffmpeg",
        "-y",
        "-i",
        str(input_path.resolve()),
        "-vf",
        f"scale=-2:{target_height}",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "28",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        str(output_path.resolve()),
    ]

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=300,
            check=False,
        )
    except subprocess.TimeoutExpired as e:
        raise FFmpegError(f"Proxy generation timed out for {input_path}") from e
    except FileNotFoundError as e:
        raise FileNotFoundError("ffmpeg executable not found in PATH.") from e

    if result.returncode != 0:
        raise FFmpegError(
            f"ffmpeg failed with exit code {result.returncode} on {input_path}.\n"
            f"Stderr: {result.stderr}"
        )


def extract_thumbnail(input_path: Path, output_path: Path, timestamp: float) -> None:
    """Extract a single frame from the input video at timestamp as a JPEG thumbnail.

    Args:
        input_path: Path to the source video file.
        output_path: Target path for the thumbnail JPEG.
        timestamp: Time offset in seconds to capture the frame.

    Raises:
        FileNotFoundError: If ffmpeg is not in PATH.
        FFmpegError: If ffmpeg process fails or times out.
    """
    if not shutil.which("ffmpeg"):
        raise FileNotFoundError("ffmpeg executable not found in PATH.")

    output_path.parent.mkdir(parents=True, exist_ok=True)

    cmd = [
        "ffmpeg",
        "-y",
        "-ss",
        f"{timestamp:.3f}",
        "-i",
        str(input_path.resolve()),
        "-vframes",
        "1",
        "-vf",
        "scale=160:90:force_original_aspect_ratio=decrease",
        str(output_path.resolve()),
    ]

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=300,
            check=False,
        )
    except subprocess.TimeoutExpired as e:
        raise FFmpegError(f"Thumbnail extraction timed out for {input_path}") from e
    except FileNotFoundError as e:
        raise FileNotFoundError("ffmpeg executable not found in PATH.") from e

    if result.returncode != 0:
        raise FFmpegError(
            f"ffmpeg failed with exit code {result.returncode} on {input_path}.\n"
            f"Stderr: {result.stderr}"
        )


def extract_waveform_peaks(input_path: Path) -> list[tuple[float, float]]:
    """Extract audio waveform peaks by decoding audio to 8kHz mono 16-bit PCM.

    Args:
        input_path: Path to the media file.

    Returns:
        A list of (min_peak, max_peak) normalized float values at ~100 samples/sec.
    """
    # 1. First probe to verify if file actually has audio stream
    try:
        info = probe(input_path)
    except Exception:
        return []

    has_audio = any(s.get("codec_type") == "audio" for s in info.get("streams", []))
    if not has_audio:
        return []

    if not shutil.which("ffmpeg"):
        raise FileNotFoundError("ffmpeg executable not found in PATH.")

    # Resample to 8kHz, mono, s16le PCM to be piped out
    cmd = [
        "ffmpeg",
        "-y",
        "-i",
        str(input_path.resolve()),
        "-map",
        "0:a",
        "-ac",
        "1",
        "-filter:a",
        "aresample=8000",
        "-c:a",
        "pcm_s16le",
        "-f",
        "s16le",
        "-",
    ]

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            timeout=300,
            check=False,
        )
    except subprocess.TimeoutExpired as e:
        raise FFmpegError(f"Waveform extraction timed out for {input_path}") from e
    except FileNotFoundError as e:
        raise FileNotFoundError("ffmpeg executable not found in PATH.") from e

    if result.returncode != 0:
        # Standard fallback if mapping fails or encoding fails
        return []

    raw_data = result.stdout
    num_samples = len(raw_data) // 2
    if num_samples == 0:
        return []

    # Unpack as signed short integers (16-bit)
    samples = struct.unpack(f"{num_samples}h", raw_data)

    peaks: list[tuple[float, float]] = []
    # 8000 samples/sec, we want 100 peak points/sec => chunk size of 80 samples
    chunk_size = 80

    for i in range(0, len(samples), chunk_size):
        chunk = samples[i : i + chunk_size]
        if not chunk:
            continue
        min_val = min(chunk) / 32768.0
        max_val = max(chunk) / 32768.0
        peaks.append((min_val, max_val))

    return peaks


def scale_image(input_path: Path, output_path: Path) -> None:
    """Scale a static image file using FFmpeg for thumbnail extraction.

    Args:
        input_path: Path to the source image file.
        output_path: Target path for the scaled thumbnail JPEG.

    Raises:
        FileNotFoundError: If ffmpeg is not in PATH.
        FFmpegError: If the process fails or times out.
    """
    if not shutil.which("ffmpeg"):
        raise FileNotFoundError("ffmpeg executable not found in PATH.")

    output_path.parent.mkdir(parents=True, exist_ok=True)

    cmd = [
        "ffmpeg",
        "-y",
        "-i",
        str(input_path.resolve()),
        "-vf",
        "scale=160:90:force_original_aspect_ratio=decrease",
        str(output_path.resolve()),
    ]

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=300,
            check=False,
        )
    except subprocess.TimeoutExpired as e:
        raise FFmpegError(f"Image scaling timed out for {input_path}") from e
    except FileNotFoundError as e:
        raise FileNotFoundError("ffmpeg executable not found in PATH.") from e

    if result.returncode != 0:
        raise FFmpegError(
            f"ffmpeg image scaling failed with exit code {result.returncode} on {input_path}.\n"
            f"Stderr: {result.stderr}"
        )
