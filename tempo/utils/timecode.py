"""Timecode formatting and parsing helpers for the Tempo video editor.

Supports standard SMPTE timecode format: HH:MM:SS:FF, where FF is the frame number.
"""


def seconds_to_timecode(seconds: float, fps: float) -> str:
    """Convert a duration in seconds to a timecode string (HH:MM:SS:FF).

    Args:
        seconds: Duration in seconds. Must be non-negative.
        fps: Frames per second. Must be positive.

    Returns:
        Timecode string in the format HH:MM:SS:FF.
    """
    if seconds < 0:
        seconds = 0.0
    if fps <= 0:
        raise ValueError("FPS must be positive.")

    # Calculate total frames by rounding to nearest frame to avoid float inaccuracies
    total_frames = round(seconds * fps)

    fps_int = round(fps)
    frames = total_frames % fps_int

    total_seconds = total_frames // fps_int
    secs = int(total_seconds % 60)
    mins = int((total_seconds // 60) % 60)
    hours = int(total_seconds // 3600)

    return f"{hours:02d}:{mins:02d}:{secs:02d}:{frames:02d}"


def timecode_to_seconds(timecode: str, fps: float) -> float:
    """Convert a timecode string (HH:MM:SS:FF) to seconds.

    Args:
        timecode: Timecode string in format HH:MM:SS:FF.
        fps: Frames per second. Must be positive.

    Returns:
        Duration in seconds.
    """
    if fps <= 0:
        raise ValueError("FPS must be positive.")

    parts = timecode.split(":")
    if len(parts) != 4:
        raise ValueError(f"Invalid timecode format: '{timecode}'. Expected HH:MM:SS:FF.")

    try:
        hours = int(parts[0])
        mins = int(parts[1])
        secs = int(parts[2])
        frames = int(parts[3])
    except ValueError as e:
        raise ValueError(f"Timecode parts must be integers: '{timecode}'.") from e

    total_frames = (hours * 3600 + mins * 60 + secs) * round(fps) + frames
    return float(total_frames) / fps


def frames_to_timecode(frames: int, fps: float) -> str:
    """Convert total frames to a timecode string (HH:MM:SS:FF).

    Args:
        frames: Total number of frames. Must be non-negative.
        fps: Frames per second. Must be positive.

    Returns:
        Timecode string in format HH:MM:SS:FF.
    """
    if frames < 0:
        frames = 0
    return seconds_to_timecode(float(frames) / fps, fps)


def timecode_to_frames(timecode: str, fps: float) -> int:
    """Convert a timecode string (HH:MM:SS:FF) to total frames.

    Args:
        timecode: Timecode string in format HH:MM:SS:FF.
        fps: Frames per second. Must be positive.

    Returns:
        Total number of frames.
    """
    seconds = timecode_to_seconds(timecode, fps)
    return round(seconds * fps)
