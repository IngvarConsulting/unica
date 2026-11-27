from __future__ import annotations

import os
import subprocess
from pathlib import Path


def windows_engine_bytes() -> bytes:
    # Use a native .exe with no private runtime libraries to stage.
    return (Path(os.environ["SystemRoot"]) / "System32" / "cmd.exe").read_bytes()


def run_windows_engine(entrypoint: Path) -> str:
    return subprocess.run(
        [str(entrypoint), "/d", "/c", "echo", "assessment-engine"],
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    ).stdout.strip()
