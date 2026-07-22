"""Project file save/load utilities for Tempo.

Handles JSON serialization/deserialization and version validation.
"""

import json
from dataclasses import asdict
from pathlib import Path
from typing import Any, Literal, cast

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


class ProjectValidationError(Exception):
    """Raised when project validation fails during loading."""


class ProjectVersionError(ProjectValidationError):
    """Raised when the project file version is incompatible."""


def save_project(project: Project, filepath: str | Path) -> None:
    """Save a Project instance to a JSON file.

    Args:
        project: The Project instance to save.
        filepath: File path to save the project JSON.
    """
    path = Path(filepath)
    # Ensure parent directory exists
    path.parent.mkdir(parents=True, exist_ok=True)

    # Convert project dataclass to dict and dump as JSON
    project_dict = asdict(project)

    with open(path, "w", encoding="utf-8") as f:
        json.dump(project_dict, f, indent=2, ensure_ascii=False)


def load_project(filepath: str | Path) -> Project:
    """Load a Project instance from a JSON file with validation.

    Args:
        filepath: File path to the project JSON.

    Returns:
        A validated Project instance.

    Raises:
        ProjectValidationError: If the JSON is invalid or missing required fields.
        ProjectVersionError: If the project version is incompatible.
    """
    path = Path(filepath)
    if not path.exists():
        raise FileNotFoundError(f"Project file not found: {path}")

    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except json.JSONDecodeError as e:
        raise ProjectValidationError(f"Invalid JSON format: {e}") from e

    if not isinstance(data, dict):
        raise ProjectValidationError("Project root must be a JSON object.")

    # 1. Version Validation
    version = data.get("version")
    if not version:
        raise ProjectValidationError("Missing project version.")
    if version != "1.0":
        raise ProjectVersionError(f"Incompatible project version: {version}. Expected '1.0'.")

    try:
        # Deserialize settings
        settings_data = data.get("settings", {})
        res_list = settings_data.get("resolution", [1920, 1080])
        resolution = (int(res_list[0]), int(res_list[1]))
        settings = ProjectSettings(
            resolution=resolution,
            framerate=float(settings_data.get("framerate", 30.0)),
            sample_rate=int(settings_data.get("sample_rate", 48000)),
        )

        # Deserialize media items
        media: list[MediaItem] = []
        for item in data.get("media", []):
            media.append(
                MediaItem(
                    id=str(item["id"]),
                    original_path=str(item["original_path"]),
                    proxy_path=item.get("proxy_path"),
                    type=item["type"],  # Literal validation done by mypy / typing
                    duration=float(item["duration"]),
                    width=int(item["width"]) if item.get("width") is not None else None,
                    height=int(item["height"]) if item.get("height") is not None else None,
                    fps=float(item["fps"]) if item.get("fps") is not None else None,
                    has_audio=bool(item.get("has_audio", False)),
                    thumbnail_path=item.get("thumbnail_path"),
                )
            )

        # Deserialize timeline
        timeline_data = data.get("timeline", {})
        tracks: list[Track] = []
        for track_data in timeline_data.get("tracks", []):
            clips: list[Clip] = []
            for clip_data in track_data.get("clips", []):
                # Helper for transitions
                def make_transition(t_data: dict[str, Any] | None) -> TransitionConfig | None:
                    if not t_data:
                        return None
                    return TransitionConfig(
                        type=str(t_data["type"]),
                        duration=float(t_data.get("duration", 1.0)),
                    )

                clips.append(
                    Clip(
                        id=str(clip_data["id"]),
                        media_id=str(clip_data["media_id"]),
                        source_path=str(clip_data.get("source_path", "")),
                        timeline_start=float(clip_data["timeline_start"]),
                        timeline_end=float(clip_data["timeline_end"]),
                        source_in=float(clip_data["source_in"]),
                        source_out=float(clip_data["source_out"]),
                        speed=float(clip_data.get("speed", 1.0)),
                        pitch_correction=bool(clip_data.get("pitch_correction", True)),
                        transition_in=make_transition(clip_data.get("transition_in")),
                        transition_out=make_transition(clip_data.get("transition_out")),
                    )
                )

            tracks.append(
                Track(
                    id=str(track_data["id"]),
                    type=track_data["type"],
                    index=int(track_data["index"]),
                    clips=clips,
                )
            )

        # Deserialize text clips
        text_clips: list[TextClip] = []
        for tc_data in timeline_data.get("text_clips", []):
            text_clips.append(
                TextClip(
                    id=str(tc_data["id"]),
                    timeline_start=float(tc_data["timeline_start"]),
                    timeline_end=float(tc_data["timeline_end"]),
                    content=str(tc_data["content"]),
                    font_family=str(tc_data.get("font_family", "Inter")),
                    font_size=int(tc_data.get("font_size", 36)),
                    font_color=str(tc_data.get("font_color", "#FFFFFF")),
                    background_color=str(tc_data.get("background_color", "#000000")),
                    background_opacity=float(tc_data.get("background_opacity", 0.0)),
                    bold=bool(tc_data.get("bold", False)),
                    italic=bool(tc_data.get("italic", False)),
                    underline=bool(tc_data.get("underline", False)),
                    alignment=cast(
                        "Literal['left', 'center', 'right']", tc_data.get("alignment", "center")
                    ),
                    position_x=float(tc_data.get("position_x", 50.0)),
                    position_y=float(tc_data.get("position_y", 85.0)),
                    rotation=float(tc_data.get("rotation", 0.0)),
                )
            )

        timeline = Timeline(
            duration=float(timeline_data.get("duration", 0.0)),
            tracks=tracks,
            text_clips=text_clips,
        )

        return Project(
            project_name=str(data["project_name"]),
            settings=settings,
            created_at=str(data["created_at"]),
            modified_at=str(data["modified_at"]),
            version=version,
            media=media,
            timeline=timeline,
        )

    except KeyError as e:
        raise ProjectValidationError(f"Missing required field in project JSON: {e}") from e
    except (ValueError, TypeError) as e:
        raise ProjectValidationError(f"Invalid value type in project JSON: {e}") from e
