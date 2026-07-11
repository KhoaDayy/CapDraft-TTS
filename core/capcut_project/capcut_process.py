"""Detect whether CapCut is running (best-effort, Windows)."""

from __future__ import annotations

import sys


def is_capcut_running() -> bool:
    names = {
        "capcut.exe",
        "jianyingpro.exe",
        "videoeditor.exe",
        "capcut",
        "jianyingpro",
    }
    if sys.platform != "win32":
        try:
            import subprocess

            out = subprocess.check_output(["ps", "-A"], text=True, errors="replace")
            lower = out.lower()
            return any(n in lower for n in names)
        except Exception:
            return False

    try:
        import subprocess

        # tasklist is available on Windows
        out = subprocess.check_output(
            ["tasklist", "/FO", "CSV", "/NH"],
            text=True,
            errors="replace",
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        lower = out.lower()
        return any(n in lower for n in ("capcut.exe", "jianyingpro.exe", "videoeditor.exe"))
    except Exception:
        try:
            import ctypes
            from ctypes import wintypes

            # fallback: EnumProcesses is heavy; just return False if tasklist fails
            return False
        except Exception:
            return False
