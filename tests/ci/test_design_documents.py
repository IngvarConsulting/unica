"""Repository hygiene for local session scratch files."""

from __future__ import annotations

import subprocess
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


class LayoutTests(unittest.TestCase):
    def test_session_scratch_is_never_tracked(self) -> None:
        """`.session-temp/` is ignored on purpose; `git add -f` defeats that."""
        result = subprocess.run(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", ".session-temp/"],
            cwd=REPO_ROOT,
            capture_output=True,
            check=True,
        )
        # Inspect the index and non-ignored paths, including missing indexed
        # files and nested repositories; ignored proof trees need no traversal.
        tracked_or_unignored = sorted(
            path.decode("utf-8", errors="surrogateescape")
            for path in result.stdout.split(b"\0")
            if path
        )
        self.assertEqual(tracked_or_unignored, [])


if __name__ == "__main__":
    unittest.main()
