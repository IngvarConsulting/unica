"""Contract test for the architecture-v1 fate ledger."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
GUARD = REPO_ROOT / "scripts" / "arch" / "fate.py"


COMPLETE_FATE = """# Fate

| Subject | Fate | Successor |
| --- | --- | --- |
| `ADR-0001` | `retired` | — |
| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` |
| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` |
| `acceptance/runtime.md` | `carried` | `CTR.WIRE.RUNTIME` |
"""


class Fixture:
    def __init__(self, stack: tempfile.TemporaryDirectory[str]) -> None:
        self.root = Path(stack.name)
        archive = self.root / "docs" / "arch-v1"
        (archive / "decisions").mkdir(parents=True)
        (archive / "architecture").mkdir()
        (archive / "acceptance").mkdir()
        (archive / "decisions" / "0001-one.md").write_text("# ADR-0001\n", encoding="utf-8")
        (archive / "architecture" / "invariants.md").write_text(
            "### INV-APP-BOUNDARY — Boundary\n", encoding="utf-8"
        )
        (archive / "architecture" / "quality-requirements.md").write_text(
            "### REQ-PERF-DEADLINE — Deadline\n", encoding="utf-8"
        )
        (archive / "acceptance" / "runtime.md").write_text("# Runtime\n", encoding="utf-8")
        (archive / "FATE.md").write_text(COMPLETE_FATE, encoding="utf-8")

        (self.root / "arch" / "invariants").mkdir(parents=True)
        (self.root / "arch" / "contracts").mkdir()
        (self.root / "arch" / "invariants" / "INV.APP.BOUNDARY.md").write_text(
            "---\nid: INV.APP.BOUNDARY\n---\n", encoding="utf-8"
        )
        (self.root / "arch" / "contracts" / "CTR.WIRE.RUNTIME.md").write_text(
            "---\nid: CTR.WIRE.RUNTIME\n---\n", encoding="utf-8"
        )

    @property
    def fate(self) -> Path:
        return self.root / "docs" / "arch-v1" / "FATE.md"

    def run(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(GUARD), "--root", str(self.root)],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )


class FateCoverageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = tempfile.TemporaryDirectory()
        self.addCleanup(self.stack.cleanup)
        self.fixture = Fixture(self.stack)

    def test_a_complete_fate_ledger_passes(self) -> None:
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_missing_v1_subject_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` |\n", ""
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REQ-PERF-DEADLINE", result.stderr)

    def test_a_duplicate_v1_subject_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE + "| `ADR-0001` | `retired` | — |\n", encoding="utf-8"
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ADR-0001", result.stderr)
        self.assertIn("duplicate", result.stderr.lower())

    def test_an_unknown_fate_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace("`retired`", "`alive`", 1), encoding="utf-8"
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("alive", result.stderr)

    def test_an_unresolved_successor_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace("INV.APP.BOUNDARY", "INV.APP.MISSING", 1),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV.APP.MISSING", result.stderr)

    def test_every_v1_subject_has_exactly_one_fate(self) -> None:
        """A moved ADR, rule, requirement, or acceptance contract cannot disappear."""
        result = subprocess.run(
            [sys.executable, str(GUARD), "--root", str(REPO_ROOT)],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
