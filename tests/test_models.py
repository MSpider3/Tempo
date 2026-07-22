"""Unit tests for the Tempo core data models."""

from tempo.core.models import (
    Clip,
    MediaItem,
    Project,
    ProjectSettings,
    TextClip,
    Timeline,
    Track,
    TransitionConfig,
)


def test_transition_config() -> None:
    """Test TransitionConfig initialization and default values."""
    config = TransitionConfig(type="cross_dissolve")
    assert config.type == "cross_dissolve"
    assert config.duration == 1.0

    config_custom = TransitionConfig(type="fade_in", duration=2.5)
    assert config_custom.type == "fade_in"
    assert config_custom.duration == 2.5


def test_clip_model() -> None:
    """Test Clip initialization and defaults."""
    transition = TransitionConfig(type="fade_in", duration=0.5)
    clip = Clip(
        id="clip-1",
        media_id="media-1",
        source_path="mock/path.mp4",
        timeline_start=10.0,
        timeline_end=20.0,
        source_in=0.0,
        source_out=10.0,
        speed=2.0,
        pitch_correction=False,
        transition_in=transition,
    )
    assert clip.id == "clip-1"
    assert clip.media_id == "media-1"
    assert clip.timeline_start == 10.0
    assert clip.timeline_end == 20.0
    assert clip.source_in == 0.0
    assert clip.source_out == 10.0
    assert clip.speed == 2.0
    assert not clip.pitch_correction
    assert clip.transition_in == transition
    assert clip.transition_out is None


def test_track_model() -> None:
    """Test Track initialization and defaults."""
    track = Track(id="V1", type="video", index=0)
    assert track.id == "V1"
    assert track.type == "video"
    assert track.index == 0
    assert isinstance(track.clips, list)
    assert len(track.clips) == 0


def test_text_clip_model() -> None:
    """Test TextClip initialization and default properties."""
    tc = TextClip(
        id="txt-1",
        timeline_start=5.0,
        timeline_end=15.0,
        content="Subtitles",
    )
    assert tc.id == "txt-1"
    assert tc.timeline_start == 5.0
    assert tc.timeline_end == 15.0
    assert tc.content == "Subtitles"
    assert tc.font_family == "Inter"
    assert tc.font_size == 36
    assert tc.font_color == "#FFFFFF"
    assert tc.background_color == "#000000"
    assert tc.background_opacity == 0.0
    assert not tc.bold
    assert not tc.italic
    assert not tc.underline
    assert tc.alignment == "center"
    assert tc.position_x == 50.0
    assert tc.position_y == 85.0
    assert tc.rotation == 0.0


def test_media_item_model() -> None:
    """Test MediaItem initialization and default values."""
    item = MediaItem(
        id="item-1",
        original_path="/path/to/video.mp4",
        proxy_path=None,
        type="video",
        duration=120.5,
        width=1920,
        height=1080,
        fps=29.97,
        has_audio=True,
    )
    assert item.id == "item-1"
    assert item.original_path == "/path/to/video.mp4"
    assert item.proxy_path is None
    assert item.type == "video"
    assert item.duration == 120.5
    assert item.width == 1920
    assert item.height == 1080
    assert item.fps == 29.97
    assert item.has_audio
    assert item.thumbnail_path is None


def test_project_settings() -> None:
    """Test ProjectSettings initialization and defaults."""
    settings = ProjectSettings()
    assert settings.resolution == (1920, 1080)
    assert settings.framerate == 30.0
    assert settings.sample_rate == 48000


def test_timeline_model() -> None:
    """Test Timeline container defaults."""
    timeline = Timeline()
    assert timeline.duration == 0.0
    assert len(timeline.tracks) == 0
    assert len(timeline.text_clips) == 0


def test_project_model() -> None:
    """Test Project model nesting and fields."""
    settings = ProjectSettings()
    timeline = Timeline()
    project = Project(
        project_name="Test Project",
        settings=settings,
        created_at="2026-07-20T01:00:00Z",
        modified_at="2026-07-20T01:05:00Z",
        timeline=timeline,
    )
    assert project.project_name == "Test Project"
    assert project.version == "1.0"
    assert project.settings == settings
    assert project.timeline == timeline
    assert len(project.media) == 0
