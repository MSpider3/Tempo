"""Unit tests for the app logger configuration."""

import logging
from pathlib import Path

from tempo.utils.logger import get_logger, setup_logger


def test_setup_logger_console_and_file(tmp_path: Path) -> None:
    """Test logger setup produces handlers and writes to file."""
    log_file = tmp_path / "tempo_test.log"
    logger = setup_logger(name="tempo_test", log_file=log_file, level=logging.DEBUG)

    assert logger.name == "tempo_test"
    assert len(logger.handlers) >= 1

    # Log some messages
    logger.info("Test info message")
    logger.debug("Test debug message")

    # File should exist and contain logged strings
    assert log_file.exists()
    content = log_file.read_text(encoding="utf-8")
    assert "[INFO] tempo_test" in content
    assert "Test info message" in content
    assert "[DEBUG] tempo_test" in content
    assert "Test debug message" in content


def test_get_child_logger() -> None:
    """Test get_logger returns a correctly scoped child logger."""
    child = get_logger("core")
    assert child.name == "tempo.core"
