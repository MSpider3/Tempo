"""Thumbnail extraction logic for Tempo media items.

Performs thumbnail extraction in a background thread-safe manner without Qt dependencies.
"""

from pathlib import Path

from tempo.core.models import MediaItem
from tempo.utils.cache import CacheManager
from tempo.utils.ffmpeg import extract_thumbnail, scale_image


class ThumbnailExtractor:
    """Extracts or scales thumbnails for MediaItems using the CacheManager and FFmpeg."""

    def extract(self, media_item: MediaItem, cache: CacheManager) -> Path:
        """Check cache for existing thumbnail or extract a new one if missing.

        Args:
            media_item: The media item to extract a thumbnail for.
            cache: The CacheManager instance to use.

        Returns:
            The Path to the extracted thumbnail JPEG file.

        Raises:
            ValueError: If media type is audio, which has no thumbnail.
        """
        orig_path = Path(media_item.original_path)

        if media_item.type == "audio":
            raise ValueError("Audio files do not have visual thumbnails.")

        key = cache.cache_key(orig_path)
        thumb_path = cache.thumb_path(key)

        if not cache.thumb_exists(key):
            if media_item.type == "image":
                # For static images, scale the image itself to thumbnail size
                scale_image(orig_path, thumb_path)
            else:
                # For videos, extract a frame at 10% in or 1 second, whichever is smaller
                timestamp = min(1.0, media_item.duration * 0.1)
                extract_thumbnail(orig_path, thumb_path, timestamp)

        return thumb_path
