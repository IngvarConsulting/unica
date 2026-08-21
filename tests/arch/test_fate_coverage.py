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

| Subject | Fate | Successor | Reason |
| --- | --- | --- | --- |
| `ADR-0001` | `retired` | — | `historical-only` |
| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |
| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |
| `acceptance/runtime.md` | `carried` | `CTR.WIRE.RUNTIME` | — |
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
            """### INV-APP-BOUNDARY — Boundary

- **Rule:** The generic application boundary remains independent of tool names.
- **Check:** `ci-test` — `tests/checks.py::test_boundary`
""",
            encoding="utf-8",
        )
        (archive / "architecture" / "quality-requirements.md").write_text(
            "### REQ-PERF-DEADLINE — Deadline\n", encoding="utf-8"
        )
        (archive / "acceptance" / "runtime.md").write_text("# Runtime\n", encoding="utf-8")
        (archive / "FATE.md").write_text(COMPLETE_FATE, encoding="utf-8")

        (self.root / "arch" / "invariants").mkdir(parents=True)
        (self.root / "arch" / "contracts").mkdir()
        (self.root / "arch" / "decisions").mkdir()
        (self.root / "arch" / "invariants" / "INV.APP.BOUNDARY.md").write_text(
            "---\nid: INV.APP.BOUNDARY\n---\n", encoding="utf-8"
        )
        (self.root / "arch" / "contracts" / "CTR.WIRE.RUNTIME.md").write_text(
            "---\nid: CTR.WIRE.RUNTIME\n---\n", encoding="utf-8"
        )
        (self.root / "tests").mkdir()
        (self.root / "tests" / "checks.py").write_text(
            "def test_boundary():\n    pass\n", encoding="utf-8"
        )

    @property
    def fate(self) -> Path:
        return self.root / "docs" / "arch-v1" / "FATE.md"

    def add_decision(self, identifier: str, *, status: str = "active") -> None:
        slug = identifier.removeprefix("DEC.").lower().replace(".", "-")
        (self.root / "arch" / "decisions" / f"{slug}.md").write_text(
            f"---\nid: {identifier}\nstatus: {status}\ngoverns: product\n---\n",
            encoding="utf-8",
        )

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
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |\n",
                "",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REQ-PERF-DEADLINE", result.stderr)

    def test_a_duplicate_v1_subject_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE
            + "| `ADR-0001` | `retired` | — | `historical-only` |\n",
            encoding="utf-8",
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

    def test_a_missing_retirement_reason_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace("`historical-only`", "—", 1), encoding="utf-8"
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ADR-0001", result.stderr)
        self.assertIn("reason", result.stderr.lower())

    def test_a_carried_subject_cannot_name_a_retirement_reason(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | `historical-only` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("reason", result.stderr.lower())

    def test_a_rule_cannot_be_called_historical_only(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `historical-only` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("historical-only", result.stderr)

    def test_a_live_check_cannot_be_called_removed(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `check-removed` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("tests/checks.py::test_boundary", result.stderr)

    def test_check_removed_requires_an_old_check(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                "| `REQ-PERF-DEADLINE` | `retired` | — | `check-removed` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REQ-PERF-DEADLINE", result.stderr)
        self.assertIn("old check", result.stderr.lower())

    def test_a_missing_named_check_allows_check_removed(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `check-removed` |",
            ),
            encoding="utf-8",
        )
        (self.fixture.root / "tests" / "checks.py").write_text(
            "def another_check():\n    pass\n", encoding="utf-8"
        )
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_generic_rule_cannot_claim_tool_surface_retirement(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `tool-surface-bound` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("unica.*", result.stderr)

    def test_a_literal_tool_name_allows_tool_surface_retirement(self) -> None:
        invariants = (
            self.fixture.root / "docs" / "arch-v1" / "architecture" / "invariants.md"
        )
        invariants.write_text(
            invariants.read_text(encoding="utf-8").replace(
                "generic application boundary", "public `unica.meta.info` boundary"
            ),
            encoding="utf-8",
        )
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `tool-surface-bound` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_behavior_removed_requires_a_resolving_decision(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                "| `REQ-PERF-DEADLINE` | `retired` | — | `behavior-removed: DEC.2026-08-21.MISSING` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("DEC.2026-08-21.MISSING", result.stderr)
        self.assertIn("does not resolve", result.stderr)

    def test_behavior_removed_requires_an_active_decision(self) -> None:
        decision = "DEC.2026-08-21.REMOVAL"
        self.fixture.add_decision(decision, status="planned")
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                f"| `REQ-PERF-DEADLINE` | `retired` | — | `behavior-removed: {decision}` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(decision, result.stderr)
        self.assertIn("planned", result.stderr)

    def test_an_active_decision_allows_behavior_removed(self) -> None:
        decision = "DEC.2026-08-21.REMOVAL"
        self.fixture.add_decision(decision)
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                f"| `REQ-PERF-DEADLINE` | `retired` | — | `behavior-removed: {decision}` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

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
