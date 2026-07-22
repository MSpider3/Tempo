"""Shared pytest fixtures and configuration for Tempo tests."""

import subprocess
from pathlib import Path

import pytest

from tempo.utils.ffmpeg import check_ffmpeg

# Helper to verify FFmpeg/FFprobe availability
FFMPEG_AVAILABLE, FFPROBE_AVAILABLE = check_ffmpeg()


@pytest.fixture(scope="session")
def ffmpeg_available() -> bool:
    """Fixture to check if FFmpeg and FFprobe are available."""
    return FFMPEG_AVAILABLE and FFPROBE_AVAILABLE


@pytest.fixture
def synthetic_video(tmp_path: Path) -> Path:
    """Fixture to generate a small synthetic video with audio using FFmpeg."""
    output_file = tmp_path / "synthetic_test.mp4"
    if FFMPEG_AVAILABLE and FFPROBE_AVAILABLE:
        # Generate 3-second 320x240 video at 30 fps with 440 Hz audio tone
        cmd = [
            "ffmpeg",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=3:size=320x240:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            str(output_file.resolve()),
        ]
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, f"Failed to generate synthetic video: {result.stderr}"
    return output_file


@pytest.fixture
def synthetic_image(tmp_path: Path) -> Path:
    """Fixture to generate a small synthetic image using FFmpeg."""
    output_file = tmp_path / "synthetic_image.jpg"
    if FFMPEG_AVAILABLE and FFPROBE_AVAILABLE:
        # Generate single JPEG frame
        cmd = [
            "ffmpeg",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=320x240:rate=1",
            "-vframes",
            "1",
            str(output_file.resolve()),
        ]
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, f"Failed to generate synthetic image: {result.stderr}"
    return output_file
