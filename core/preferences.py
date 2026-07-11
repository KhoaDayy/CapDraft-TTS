"""Normalize user-facing appearance preferences."""

from __future__ import annotations


def normalize_theme_mode(value: object) -> str:
    mode = str(value or "auto").strip().lower()
    return mode if mode in {"auto", "light", "dark"} else "auto"
