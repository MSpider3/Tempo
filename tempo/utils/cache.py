"""Cache manager for Tempo proxies, thumbnails, and waveforms.

Handles generation of deterministic cache keys and retrieves paths for cached assets.
"""

import contextlib
import hashlib
import os
from pathlib import Path


class CacheManager:
    """Manages cache directories and paths for video proxies, thumbnails, and waveforms."""

    def __init__(self, cache_root: Path | None = None) -> None:
        """Initialize CacheManager.

        Args:
            cache_root: Base path for caching. Defaults to ~/.tempo if None.
        """
        if cache_root is None:
            self.root_dir = Path(os.path.expanduser("~")) / ".tempo"
        else:
            self.root_dir = Path(cache_root)

        self.proxies_dir = self.root_dir / "proxies"
        self.thumbs_dir = self.root_dir / "thumbs"
        self.waveforms_dir = self.root_dir / "waveforms"

        # Create directories on first use
        self._ensure_dirs()

    def _ensure_dirs(self) -> None:
        """Create cache directories if they do not exist."""
        self.proxies_dir.mkdir(parents=True, exist_ok=True)
        self.thumbs_dir.mkdir(parents=True, exist_ok=True)
        self.waveforms_dir.mkdir(parents=True, exist_ok=True)

    def cache_key(self, path: Path) -> str:
        """Generate a deterministic SHA-256 hash representing a file's state.

        Hash is based on the absolute resolved path, file size, and modification time.

        Args:
            path: Path to the media file.

        Returns:
            The hex digest string of the generated hash.
        """
        resolved_path = path.resolve()
        stat = resolved_path.stat()
        key_str = f"{resolved_path}:{stat.st_size}:{stat.st_mtime}"
        return hashlib.sha256(key_str.encode("utf-8")).hexdigest()

    def proxy_path(self, key: str) -> Path:
        """Get the expected path for a cached proxy file.

        Args:
            key: The cache key.

        Returns:
            The Path where the proxy should reside.
        """
        return self.proxies_dir / f"{key}.mp4"

    def thumb_path(self, key: str) -> Path:
        """Get the expected path for a cached thumbnail file.

        Args:
            key: The cache key.

        Returns:
            The Path where the thumbnail should reside.
        """
        return self.thumbs_dir / f"{key}.jpg"

    def waveform_path(self, key: str) -> Path:
        """Get the expected path for a cached waveform JSON file.

        Args:
            key: The cache key.

        Returns:
            The Path where the waveform JSON should reside.
        """
        return self.waveforms_dir / f"{key}.json"

    def proxy_exists(self, key: str) -> bool:
        """Check if a proxy file exists in the cache for the given key.

        Args:
            key: The cache key.

        Returns:
            True if the proxy exists, False otherwise.
        """
        return self.proxy_path(key).exists()

    def thumb_exists(self, key: str) -> bool:
        """Check if a thumbnail file exists in the cache for the given key.

        Args:
            key: The cache key.

        Returns:
            True if the thumbnail exists, False otherwise.
        """
        return self.thumb_path(key).exists()

    def waveform_exists(self, key: str) -> bool:
        """Check if a waveform JSON file exists in the cache for the given key.

        Args:
            key: The cache key.

        Returns:
            True if the waveform JSON exists, False otherwise.
        """
        return self.waveform_path(key).exists()

    def invalidate(self, key: str) -> None:
        """Delete all cached files associated with the given key.

        Args:
            key: The cache key.
        """
        for path_func in (self.proxy_path, self.thumb_path, self.waveform_path):
            path = path_func(key)
            if path.exists():
                with contextlib.suppress(OSError):
                    path.unlink()

    def cleanup_orphans(self, active_keys: set[str]) -> int:
        """Delete cached files whose keys are not in the active set.

        Args:
            active_keys: Set of active cache key strings currently in use.

        Returns:
            Number of files deleted from the cache.
        """
        deleted_count = 0
        for folder in (self.proxies_dir, self.thumbs_dir, self.waveforms_dir):
            if not folder.exists():
                continue
            for filepath in folder.iterdir():
                # The file stem matches the cache key (e.g. key.mp4 -> stem is key)
                if filepath.is_file() and filepath.stem not in active_keys:
                    try:
                        filepath.unlink()
                        deleted_count += 1
                    except OSError:
                        pass
        return deleted_count
