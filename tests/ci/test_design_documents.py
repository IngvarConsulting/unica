"""Repository hygiene for local session scratch files."""

from __future__ import annotations

import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


class LayoutTests(unittest.TestCase):
    def test_session_scratch_is_never_tracked(self) -> None:
        """`.session-temp/` is ignored on purpose; `git add -f` defeats that."""
        tracked = sorted((REPO_ROOT / ".session-temp").rglob("*")) if (
            REPO_ROOT / ".session-temp"
        ).exists() else []
        tracked_files = [
            p.relative_to(REPO_ROOT).as_posix()
            for p in tracked
            if p.is_file() and not self.is_ignored(p)
        ]
        self.assertEqual(tracked_files, [])

    @staticmethod
    def is_ignored(path: Path) -> bool:
        import subprocess

        result = subprocess.run(
            ["git", "check-ignore", "-q", str(path)],
            cwd=REPO_ROOT,
            capture_output=True,
        )
        return result.returncode == 0


if __name__ == "__main__":
    unittest.main()
