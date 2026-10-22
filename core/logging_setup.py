"""Rotating file logging for SyncUp."""

from __future__ import annotations

import logging
from logging.handlers import RotatingFileHandler
from pathlib import Path

from core.storage import get_log_dir

_LOGGER_NAME = "syncup"


def setup_logging() -> tuple[logging.Logger, Path]:
    """Configure and return the application logger and its directory."""
    log_dir = get_log_dir()
    logger = logging.getLogger(_LOGGER_NAME)
    logger.setLevel(logging.INFO)
    logger.propagate = False

    if not logger.handlers:
        handler = RotatingFileHandler(
            log_dir / "syncup.log",
            maxBytes=2_000_000,
            backupCount=3,
            encoding="utf-8",
        )
        handler.setFormatter(
            logging.Formatter("%(asctime)s %(levelname)-8s %(message)s")
        )
        logger.addHandler(handler)

    return logger, log_dir
