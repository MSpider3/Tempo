"""Unit tests for timecode formatting and parsing."""

import pytest

from tempo.utils.timecode import (
    frames_to_timecode,
    seconds_to_timecode,
    timecode_to_frames,
    timecode_to_seconds,
)


def test_seconds_to_timecode() -> None:
    """Test converting duration in seconds to HH:MM:SS:FF."""
    # Simple cases at 30 fps
    assert seconds_to_timecode(0.0, 30.0) == "00:00:00:00"
    assert seconds_to_timecode(1.0, 30.0) == "00:00:01:00"
    assert seconds_to_timecode(59.0, 30.0) == "00:00:59:00"
    assert seconds_to_timecode(60.0, 30.0) == "00:01:00:00"
    assert seconds_to_timecode(3599.0, 30.0) == "00:59:59:00"
    assert seconds_to_timecode(3600.0, 30.0) == "01:00:00:00"

    # Frame calculations at 30 fps
    assert seconds_to_timecode(0.033333, 30.0) == "00:00:00:01"
    assert seconds_to_timecode(0.5, 30.0) == "00:00:00:15"
    assert seconds_to_timecode(0.966666, 30.0) == "00:00:00:29"

    # Frame calculations at 24 fps
    assert seconds_to_timecode(0.041667, 24.0) == "00:00:00:01"
    assert seconds_to_timecode(0.5, 24.0) == "00:00:00:12"
    assert seconds_to_timecode(0.958333, 24.0) == "00:00:00:23"

    # Negative inputs should default to 0
    assert seconds_to_timecode(-5.0, 30.0) == "00:00:00:00"


def test_timecode_to_seconds() -> None:
    """Test converting HH:MM:SS:FF timecode to seconds."""
    assert timecode_to_seconds("00:00:00:00", 30.0) == 0.0
    assert timecode_to_seconds("00:00:01:00", 30.0) == 1.0
    assert timecode_to_seconds("00:01:00:00", 30.0) == 60.0
    assert timecode_to_seconds("01:00:00:00", 30.0) == 3600.0

    # Frames at 30 fps
    assert timecode_to_seconds("00:00:00:15", 30.0) == 0.5
    assert pytest.approx(timecode_to_seconds("00:00:00:01", 30.0)) == 1.0 / 30.0

    # Invalid timecode inputs
    with pytest.raises(ValueError):
        timecode_to_seconds("00:00:00", 30.0)  # Missing frames
    with pytest.raises(ValueError):
        timecode_to_seconds("aa:00:00:00", 30.0)  # Non-integer


def test_frames_to_timecode() -> None:
    """Test converting total frame count to timecode string."""
    assert frames_to_timecode(0, 30.0) == "00:00:00:00"
    assert frames_to_timecode(15, 30.0) == "00:00:00:15"
    assert frames_to_timecode(30, 30.0) == "00:00:01:00"
    assert frames_to_timecode(1800, 30.0) == "00:01:00:00"


def test_timecode_to_frames() -> None:
    """Test converting timecode string to total frames."""
    assert timecode_to_frames("00:00:00:00", 30.0) == 0
    assert timecode_to_frames("00:00:00:15", 30.0) == 15
    assert timecode_to_frames("00:00:01:00", 30.0) == 30
    assert timecode_to_frames("00:01:00:00", 30.0) == 1800


def test_invalid_fps() -> None:
    """Test that positive FPS is enforced."""
    with pytest.raises(ValueError):
        seconds_to_timecode(1.0, 0.0)
    with pytest.raises(ValueError):
        timecode_to_seconds("00:00:00:00", -24.0)
