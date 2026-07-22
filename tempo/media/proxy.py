"""Proxy generation logic for Tempo video items.

Runs proxy generation in a background thread-safe manner without Qt dependencies.
"""

from pathlib import Path

from tempo.core.models import MediaItem
from tempo.utils.cache import CacheManager
from tempo.utils.ffmpeg import generate_proxy


def target_proxy_height(source_height: int) -> int | None:
    """Determine the target height for the proxy file based on source height.

    Args:
        source_height: Vertical resolution of the source media.

    Returns:
        The target proxy height (360 or 480) or None if no proxy is needed.
    """
    if source_height >= 2160:  # 4K and higher
        return 360
    elif source_height >= 1080:  # 1080p and higher (up to 4K)
        return 480
    else:
        return None  # Under 1080p: use original file


class ProxyGenerator:
    """Generates proxies for MediaItems using the CacheManager and FFmpeg."""

    def generate(self, media_item: MediaItem, cache: CacheManager) -> Path:
        """Check cache for existing proxy or generate a new one if missing.

        Args:
            media_item: The media item to generate a proxy for.
            cache: The CacheManager instance to use.

        Returns:
            The Path to the proxy file (or original path if no proxy is needed).
        """
        orig_path = Path(media_item.original_path)

        # Only video files need proxies
        if media_item.type != "video" or media_item.height is None:
            return orig_path

        target_height = target_proxy_height(media_item.height)
        if target_height is None:
            return orig_path

        key = cache.cache_key(orig_path)
        proxy_path = cache.proxy_path(key)

        if not cache.proxy_exists(key):
            # Run background FFmpeg generation process
            generate_proxy(orig_path, proxy_path, target_height)

        return proxy_path
