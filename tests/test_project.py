"""Unit tests for project save/load operations and JSON validation."""

import json
from pathlib import Path

import pytest

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
from tempo.core.project import (
    ProjectValidationError,
    ProjectVersionError,
    load_project,
    save_project,
)


@pytest.fixture
def sample_project() -> Project:
    """Fixture providing a rich sample project for testing."""
    settings = ProjectSettings(resolution=(1920, 1080), framerate=24.0, sample_rate=44100)

    media = [
        MediaItem(
            id="media-v1",
            original_path="/path/to/clip.mp4",
            proxy_path="/path/to/proxy.mp4",
            type="video",
            duration=60.0,
            width=1920,
            height=1080,
            fps=24.0,
            has_audio=True,
            thumbnail_path="/path/to/thumb.jpg",
        )
    ]

    clip = Clip(
        id="clip-v1",
        media_id="media-v1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=5.0,
        source_out=15.0,
        speed=1.0,
        pitch_correction=True,
        transition_in=TransitionConfig(type="fade_in", duration=0.5),
        transition_out=TransitionConfig(type="fade_out", duration=1.0),
    )

    track = Track(id="V1", type="video", index=0, clips=[clip])

    text_clip = TextClip(
        id="text-1",
        timeline_start=2.0,
        timeline_end=7.0,
        content="Hello!",
        font_family="Inter",
        font_size=48,
        font_color="#FF0000",
        bold=True,
    )

    timeline = Timeline(duration=10.0, tracks=[track], text_clips=[text_clip])

    return Project(
        project_name="Sample Project",
        settings=settings,
        created_at="2026-07-20T01:00:00Z",
        modified_at="2026-07-20T01:05:00Z",
        version="1.0",
        media=media,
        timeline=timeline,
    )


def test_save_and_load_project_roundtrip(sample_project: Project, tmp_path: Path) -> None:
    """Test saving and loading back results in the same project data."""
    project_file = tmp_path / "test_project.tempo"

    # Save
    save_project(sample_project, project_file)
    assert project_file.exists()

    # Load
    loaded = load_project(project_file)

    # Assertions
    assert loaded.project_name == sample_project.project_name
    assert loaded.version == sample_project.version
    assert loaded.created_at == sample_project.created_at
    assert loaded.modified_at == sample_project.modified_at

    # Settings
    assert loaded.settings.resolution == sample_project.settings.resolution
    assert loaded.settings.framerate == sample_project.settings.framerate
    assert loaded.settings.sample_rate == sample_project.settings.sample_rate

    # Media items
    assert len(loaded.media) == len(sample_project.media)
    assert loaded.media[0].id == sample_project.media[0].id
    assert loaded.media[0].original_path == sample_project.media[0].original_path
    assert loaded.media[0].proxy_path == sample_project.media[0].proxy_path
    assert loaded.media[0].duration == sample_project.media[0].duration
    assert loaded.media[0].fps == sample_project.media[0].fps
    assert loaded.media[0].has_audio == sample_project.media[0].has_audio

    # Timeline tracks & clips
    assert len(loaded.timeline.tracks) == len(sample_project.timeline.tracks)
    loaded_track = loaded.timeline.tracks[0]
    sample_track = sample_project.timeline.tracks[0]
    assert loaded_track.id == sample_track.id
    assert loaded_track.type == sample_track.type

    assert len(loaded_track.clips) == len(sample_track.clips)
    loaded_clip = loaded_track.clips[0]
    sample_clip = sample_track.clips[0]
    assert loaded_clip.id == sample_clip.id
    assert loaded_clip.media_id == sample_clip.media_id
    assert loaded_clip.timeline_start == sample_clip.timeline_start
    assert loaded_clip.timeline_end == sample_clip.timeline_end
    assert loaded_clip.speed == sample_clip.speed
    assert loaded_clip.pitch_correction == sample_clip.pitch_correction
    assert loaded_clip.transition_in is not None
    assert loaded_clip.transition_in.type == "fade_in"
    assert loaded_clip.transition_in.duration == 0.5
    assert loaded_clip.transition_out is not None
    assert loaded_clip.transition_out.type == "fade_out"
    assert loaded_clip.transition_out.duration == 1.0

    # Text clips
    assert len(loaded.timeline.text_clips) == len(sample_project.timeline.text_clips)
    loaded_txt = loaded.timeline.text_clips[0]
    sample_txt = sample_project.timeline.text_clips[0]
    assert loaded_txt.id == sample_txt.id
    assert loaded_txt.content == sample_txt.content
    assert loaded_txt.font_family == sample_txt.font_family
    assert loaded_txt.font_size == sample_txt.font_size
    assert loaded_txt.font_color == sample_txt.font_color
    assert loaded_txt.bold == sample_txt.bold
    assert not loaded_txt.italic


def test_load_project_file_not_found() -> None:
    """Test loading a non-existent file raises FileNotFoundError."""
    with pytest.raises(FileNotFoundError):
        load_project("non_existent_file.tempo")


def test_load_project_invalid_json(tmp_path: Path) -> None:
    """Test loading a file with corrupt JSON raises ProjectValidationError."""
    project_file = tmp_path / "corrupt.tempo"
    with open(project_file, "w") as f:
        f.write("{invalid json")

    with pytest.raises(ProjectValidationError) as excinfo:
        load_project(project_file)
    assert "Invalid JSON format" in str(excinfo.value)


def test_load_project_root_not_dict(tmp_path: Path) -> None:
    """Test loading a JSON array raises ProjectValidationError."""
    project_file = tmp_path / "array.tempo"
    with open(project_file, "w") as f:
        f.write("[1, 2, 3]")

    with pytest.raises(ProjectValidationError) as excinfo:
        load_project(project_file)
    assert "Project root must be a JSON object" in str(excinfo.value)


def test_load_project_missing_version(tmp_path: Path) -> None:
    """Test loading JSON with missing version field raises ProjectValidationError."""
    project_file = tmp_path / "no_version.tempo"
    data = {"project_name": "No Version"}
    with open(project_file, "w") as f:
        json.dump(data, f)

    with pytest.raises(ProjectValidationError) as excinfo:
        load_project(project_file)
    assert "Missing project version" in str(excinfo.value)


def test_load_project_invalid_version(tmp_path: Path) -> None:
    """Test loading JSON with incompatible version raises ProjectVersionError."""
    project_file = tmp_path / "bad_version.tempo"
    data = {"project_name": "Bad Version", "version": "2.0"}
    with open(project_file, "w") as f:
        json.dump(data, f)

    with pytest.raises(ProjectVersionError) as excinfo:
        load_project(project_file)
    assert "Incompatible project version: 2.0" in str(excinfo.value)


def test_load_project_missing_required_fields(tmp_path: Path) -> None:
    """Test missing project fields raise ProjectValidationError."""
    project_file = tmp_path / "missing_fields.tempo"
    data = {
        "version": "1.0",
        # "project_name" is missing
        "created_at": "2026-07-20T01:00:00Z",
        "modified_at": "2026-07-20T01:05:00Z",
    }
    with open(project_file, "w") as f:
        json.dump(data, f)

    with pytest.raises(ProjectValidationError) as excinfo:
        load_project(project_file)
    assert "Missing required field" in str(excinfo.value)


def test_load_project_invalid_value_types(tmp_path: Path) -> None:
    """Test that invalid value types raise a validation error."""
    project_file = tmp_path / "bad_type.tempo"
    data = {
        "version": "1.0",
        "project_name": "Bad Type",
        "created_at": "2026-07-20T01:00:00Z",
        "modified_at": "2026-07-20T01:05:00Z",
        "settings": {"resolution": [1920, 1080], "framerate": "not-a-float"},
    }
    with open(project_file, "w") as f:
        json.dump(data, f)

    with pytest.raises(ProjectValidationError) as excinfo:
        load_project(project_file)
    assert "Invalid value type" in str(excinfo.value)
