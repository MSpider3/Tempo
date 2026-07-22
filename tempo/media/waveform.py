"""Waveform extraction and caching logic for Tempo audio/video items.

Processes peak data in a background thread-safe manner without Qt dependencies.
"""

import json
from pathlib import Path

from tempo.core.models import MediaItem
from tempo.utils.cache import CacheManager
from tempo.utils.ffmpeg import extract_waveform_peaks


class WaveformExtractor:
    """Extracts and caches audio waveform peak data as JSON files."""

    def __init__(self, cache: CacheManager | None = None) -> None:
        """Initialize the WaveformExtractor.

        Args:
            cache: CacheManager instance. Creates a default one if None.
        """
        self.cache = cache or CacheManager()

    def extract(self, media_item: MediaItem) -> list[tuple[float, float]]:
        """Extract or retrieve cached waveform peak data for a MediaItem.

        Args:
            media_item: The media item containing audio.

        Returns:
            A list of (min_peak, max_peak) float tuples, or [] if no audio is present.
        """
        if not media_item.has_audio:
            return []

        orig_path = Path(media_item.original_path)
        key = self.cache.cache_key(orig_path)
        waveform_file = self.cache.waveform_path(key)

        # Check cache first
        if self.cache.waveform_exists(key):
            try:
                with open(waveform_file, encoding="utf-8") as f:
                    peaks_list = json.load(f)
                return [
                    (float(p[0]), float(p[1]))
                    for p in peaks_list
                    if isinstance(p, (list, tuple)) and len(p) == 2
                ]
            except Exception:
                # If cache is corrupt, invalidate and re-extract
                self.cache.invalidate(key)

        # Extract using FFmpeg
        peaks = extract_waveform_peaks(orig_path)

        # Write to cache
        try:
            with open(waveform_file, "w", encoding="utf-8") as f:
                json.dump(peaks, f)
        except Exception:
            pass

        return peaks
