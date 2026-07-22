"""Utility for managing recent project history in Tempo."""

from __future__ import annotations

import contextlib
import json
from pathlib import Path

RECENT_PROJECTS_PATH = Path.home() / ".tempo" / "recent_projects.json"
MAX_RECENT = 10


def load_recent_projects() -> list[Path]:
    """Load recent projects list. Returns paths that still exist on disk."""
    try:
        data = json.loads(RECENT_PROJECTS_PATH.read_text(encoding="utf-8"))
        if not isinstance(data, list):
            return []
        return [Path(p) for p in data if isinstance(p, str) and Path(p).exists()][:MAX_RECENT]
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        return []


def add_recent_project(path: Path) -> None:
    """Add path to front of recent list. Deduplicate. Save."""
    resolved_path = path.resolve()
    recent = load_recent_projects()
    recent = [p for p in recent if p.resolve() != resolved_path]
    recent.insert(0, resolved_path)
    recent = recent[:MAX_RECENT]
    with contextlib.suppress(OSError):
        RECENT_PROJECTS_PATH.parent.mkdir(parents=True, exist_ok=True)
        RECENT_PROJECTS_PATH.write_text(
            json.dumps([str(p) for p in recent], indent=2), encoding="utf-8"
        )


def clear_recent_projects() -> None:
    """Clear recent projects file from disk."""
    if RECENT_PROJECTS_PATH.exists():
        with contextlib.suppress(OSError):
            RECENT_PROJECTS_PATH.unlink()
