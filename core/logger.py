"""Application logger."""

from __future__ import annotations

import logging
import sys
from pathlib import Path

logger = logging.getLogger("capdraft_tts")
logger.setLevel(logging.DEBUG)

formatter = logging.Formatter(
    "[%(asctime)s] [%(levelname)s] [%(filename)s:%(lineno)d] - %(message)s"
)

try:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

if not logger.handlers:
    console = logging.StreamHandler(sys.stdout)
    console.setLevel(logging.INFO)
    console.setFormatter(formatter)
    logger.addHandler(console)

_file_handler: logging.FileHandler | None = None


def setup_file_logger(log_path: Path, *, mode: str = "a") -> Path:
    """Attach a UTF-8 file handler; returns the path."""
    global _file_handler
    if _file_handler:
        logger.removeHandler(_file_handler)
        _file_handler.close()
        _file_handler = None

    log_path = Path(log_path)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    handler = logging.FileHandler(log_path, mode=mode, encoding="utf-8")
    handler.setLevel(logging.DEBUG)
    handler.setFormatter(formatter)
    logger.addHandler(handler)
    _file_handler = handler
    logger.info("File logger redirected to: %s", log_path)
    return log_path
