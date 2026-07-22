"""App logging utility for the Tempo video editor.

Configures streaming and file-based logging.
"""

import logging
import os
from logging.handlers import RotatingFileHandler
from pathlib import Path


def setup_logger(
    name: str = "tempo",
    log_file: str | Path | None = None,
    level: int = logging.DEBUG,
) -> logging.Logger:
    """Set up and configure the logger with console and rotating file handlers.

    Args:
        name: Name of the logger.
        log_file: Path to the log file. Defaults to ~/.tempo/tempo.log if None.
        level: Logging level (e.g. logging.DEBUG, logging.INFO).

    Returns:
        A configured logging.Logger instance.
    """
    logger = logging.getLogger(name)
    logger.setLevel(level)

    # Avoid duplicate handlers if setup_logger is called multiple times
    if logger.handlers:
        return logger

    # Create formatter
    formatter = logging.Formatter(
        fmt="%(asctime)s [%(levelname)s] %(name)s (%(threadName)s): %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )

    # 1. Console Handler
    console_handler = logging.StreamHandler()
    console_handler.setLevel(logging.INFO)
    console_handler.setFormatter(formatter)
    logger.addHandler(console_handler)

    # 2. File Handler (with fallback if directory creation fails)
    if log_file is None:
        log_file = Path(os.path.expanduser("~")) / ".tempo" / "tempo.log"
    else:
        log_file = Path(log_file)

    try:
        log_file.parent.mkdir(parents=True, exist_ok=True)
        # 5MB max file size, keep last 3 logs
        file_handler = RotatingFileHandler(
            log_file,
            maxBytes=5 * 1024 * 1024,
            backupCount=3,
            encoding="utf-8",
        )
        file_handler.setLevel(logging.DEBUG)
        file_handler.setFormatter(formatter)
        logger.addHandler(file_handler)
    except Exception as e:
        # Fallback to console only if we cannot write to log file
        logger.warning("Failed to initialize file logger: %s. Using console only.", e)

    return logger


def get_logger(name: str) -> logging.Logger:
    """Retrieve a child logger under the 'tempo' namespace.

    Args:
        name: Name of the module or component.

    Returns:
        A logger instance.
    """
    # Ensure root "tempo" logger is configured (will use defaults if not already set up)
    logging.getLogger("tempo")
    return logging.getLogger(f"tempo.{name}")
