"""Core data models for the Tempo video editor.

All models are pure Python dataclasses, framework-agnostic (no Qt dependencies),
and fully typed for strict static analysis.
"""

from dataclasses import dataclass, field
from typing import Literal


@dataclass
class TransitionConfig:
    """Configuration for clip transitions."""

    type: str  # "cut", "fade_in", "fade_out", "cross_dissolve", etc.
    duration: float = 1.0


@dataclass
class Clip:
    """A single audio or video clip placed on a timeline track."""

    id: str
    media_id: str
    source_path: str
    timeline_start: float
    timeline_end: float
    source_in: float
    source_out: float
    speed: float = 1.0
    pitch_correction: bool = True
    transition_in: TransitionConfig | None = None
    transition_out: TransitionConfig | None = None


@dataclass
class Track:
    """A track lane containing non-overlapping clips."""

    id: str  # e.g., "V1", "V2", "A1", etc.
    type: Literal["video", "audio"]
    index: int
    clips: list[Clip] = field(default_factory=list)


@dataclass
class TextClip:
    """A text overlay clip on the dedicated TX track."""

    id: str
    timeline_start: float
    timeline_end: float
    content: str
    font_family: str = "Inter"
    font_size: int = 36
    font_color: str = "#FFFFFF"
    background_color: str = "#000000"
    background_opacity: float = 0.0  # 0.0 (transparent) to 1.0 (opaque)
    bold: bool = False
    italic: bool = False
    underline: bool = False
    alignment: Literal["left", "center", "right"] = "center"
    position_x: float = 50.0  # Percentage 0-100
    position_y: float = 85.0  # Percentage 0-100
    rotation: float = 0.0  # Degrees -180 to 180


@dataclass
class MediaItem:
    """Metadata for imported raw media assets (video, audio, or image)."""

    id: str
    original_path: str
    proxy_path: str | None
    type: Literal["video", "audio", "image"]
    duration: float
    width: int | None = None
    height: int | None = None
    fps: float | None = None
    has_audio: bool = False
    thumbnail_path: str | None = None


@dataclass
class ProjectSettings:
    """General configuration settings for a Tempo project."""

    resolution: tuple[int, int] = (1920, 1080)
    framerate: float = 30.0
    sample_rate: int = 48000


@dataclass
class Timeline:
    """Container for tracks and text clips in a project."""

    duration: float = 0.0
    tracks: list[Track] = field(default_factory=list)
    text_clips: list[TextClip] = field(default_factory=list)


@dataclass
class Project:
    """Top-level project model holding all media assets and timeline state."""

    project_name: str
    settings: ProjectSettings
    created_at: str
    modified_at: str
    version: str = "1.0"
    media: list[MediaItem] = field(default_factory=list)
    timeline: Timeline = field(default_factory=Timeline)
