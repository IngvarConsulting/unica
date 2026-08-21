"""Guards for the immutability of accepted product records.

The live tree cannot exercise this one yet: `arch/` has not reached `main`, so
a comparison against the base branch sees zero records and would report green
having looked at nothing. That is the failure mode this repository has already
paid for twice, so the rule is proved against fixtures — a real git repository
built per case — and the live check asserts what it actually compared rather
than only that it found nothing wrong.
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "arch" / "immutability.py"
SPEC = importlib.util.spec_from_file_location("arch_immutability", SCRIPT)
IMMUTABILITY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = IMMUTABILITY
SPEC.loader.exec_module(IMMUTABILITY)

PRODUCT = """---
id: DEC.2026-01-01.PROMISE
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: []
---

# Обещание

**Решение.** Поверхность отвечает так, а не иначе.
"""

PROCESS = """---
id: DEC.2026-01-01.HABIT
status: active
governs: process
realized: null
supersedes: []
superseded-by: null
establishes: []
---

# Привычка

**Решение.** Работаем так, а не иначе.
"""


RULE = """---
id: INV.WIRE.PROMISE
status: active
governs: product
decision: DEC.2026-01-01.PROMISE
check: tests/ci/test_x.py::test_y
scope: [wire]
---

# Правило

Поверхность отвечает так, а не иначе.
"""

GROUND = """---
id: DEC.2026-03-03.WHY-IT-CHANGES
status: active
governs: product
realized: tests/evidence.py::test_reason
supersedes: []
superseded-by: null
establishes: []
---

# Почему правило меняется

**Решение.** Замер показал, что прежняя формулировка не покрывала случай.
"""

INVARIANT_GROUND = """---
id: INV.WIRE.WHY-IT-CHANGES
status: active
governs: product
decision: DEC.2026-01-01.PROMISE
check: tests/evidence.py::test_reason
scope: [wire]
---

# Не решение
"""

PLANNED_GROUND = GROUND.replace("status: active", "status: planned").replace(
    "realized: tests/evidence.py::test_reason", "realized: null"
)
PROCESS_GROUND = GROUND.replace("governs: product", "governs: process")
UNREALIZED_GROUND = GROUND.replace("realized: tests/evidence.py::test_reason", "realized: null")
MISSING_EVIDENCE_GROUND = GROUND.replace("test_reason", "test_missing")


class Fixture:
    """A real git repository with a base commit holding two records."""

    def __init__(self, stack: tempfile.TemporaryDirectory) -> None:
        self.root = Path(stack.name)
        self._git("init", "--quiet", "--initial-branch=base")
        self._git("config", "user.email", "guard@example.test")
        self._git("config", "user.name", "Guard")
        (self.root / "arch" / "decisions").mkdir(parents=True)
        self.product = self.root / "arch" / "decisions" / "2026-01-01-promise.md"
        self.process = self.root / "arch" / "decisions" / "2026-01-01-habit.md"
        self.product.write_text(PRODUCT, encoding="utf-8")
        self.process.write_text(PROCESS, encoding="utf-8")
        (self.root / "arch" / "invariants").mkdir(parents=True)
        self.rule = self.root / "arch" / "invariants" / "INV.WIRE.PROMISE.md"
        self.rule.write_text(RULE, encoding="utf-8")
        (self.root / "tests").mkdir()
        (self.root / "tests" / "evidence.py").write_text(
            "def test_reason(): pass\n", encoding="utf-8"
        )
        self._git("add", "arch", "tests")
        self._git("commit", "--quiet", "--no-gpg-sign", "-m", "base")

    def _git(self, *args: str) -> None:
        subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True)

    def inspect(self):
        return IMMUTABILITY.inspect(self.root, "base")


class ProductImmutabilityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = tempfile.TemporaryDirectory()
        self.addCleanup(self.stack.cleanup)
        self.fixture = Fixture(self.stack)

    def point_rule_at(self, filename: str, text: str, identifier: str):
        target = self.fixture.root / "arch" / filename
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
        self.fixture.rule.write_text(
            RULE.replace("так, а не иначе", "уже совсем иначе").replace(
                "DEC.2026-01-01.PROMISE", identifier
            ),
            encoding="utf-8",
        )
        return self.fixture.inspect()

    def test_an_untouched_tree_is_clean_and_says_what_it_compared(self) -> None:
        verdict = self.fixture.inspect()
        self.assertEqual(verdict.offenders, ())
        self.assertEqual(
            verdict.compared, 2,
            "both product records count, and the process one does not",
        )

    def test_editing_an_accepted_product_decision_is_caught(self) -> None:
        self.fixture.product.write_text(
            PRODUCT.replace("так, а не иначе", "уже совсем иначе"), encoding="utf-8"
        )
        verdict = self.fixture.inspect()
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("отредактировано", verdict.offenders[0])

    def test_deleting_an_accepted_product_record_is_caught(self) -> None:
        self.fixture.product.unlink()
        verdict = self.fixture.inspect()
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("удалена", verdict.offenders[0])

    def test_stamping_a_supersession_is_allowed(self) -> None:
        """Replacement is the one legitimate edit, and it touches two fields."""
        stamped = PRODUCT.replace("status: planned", "status: superseded").replace(
            "superseded-by: null", "superseded-by: DEC.2026-02-02.BETTER-PROMISE"
        )
        self.fixture.product.write_text(stamped, encoding="utf-8")
        self.assertEqual(self.fixture.inspect().offenders, ())

    def test_a_supersession_stamp_may_not_smuggle_a_body_edit(self) -> None:
        stamped = (
            PRODUCT.replace("status: planned", "status: superseded")
            .replace("superseded-by: null", "superseded-by: DEC.2026-02-02.BETTER-PROMISE")
            .replace("так, а не иначе", "уже совсем иначе")
        )
        self.fixture.product.write_text(stamped, encoding="utf-8")
        self.assertEqual(len(self.fixture.inspect().offenders), 1)

    def test_supersession_status_without_successor_is_caught(self) -> None:
        self.fixture.product.write_text(
            PRODUCT.replace("status: planned", "status: superseded"), encoding="utf-8"
        )
        self.assertEqual(len(self.fixture.inspect().offenders), 1)

    def test_successor_without_supersession_status_is_caught(self) -> None:
        self.fixture.product.write_text(
            PRODUCT.replace(
                "superseded-by: null", "superseded-by: DEC.2026-02-02.BETTER-PROMISE"
            ),
            encoding="utf-8",
        )
        self.assertEqual(len(self.fixture.inspect().offenders), 1)

    def test_stamping_a_realization_is_allowed(self) -> None:
        """A planned decision becomes active atomically with its evidence."""
        stamped = (
            PRODUCT.replace("status: planned", "status: active")
            .replace(
                "realized: null",
                "realized: tests/arch/test_product_immutability.py::test_stamping_a_realization_is_allowed",
            )
        )
        self.assertNotEqual(stamped, PRODUCT, "the fixture must carry an unrealized decision")
        self.fixture.product.write_text(stamped, encoding="utf-8")
        self.assertEqual(self.fixture.inspect().offenders, ())

    def test_realization_without_activation_is_caught(self) -> None:
        stamped = PRODUCT.replace(
            "realized: null",
            "realized: tests/arch/test_product_immutability.py::test_stamping_a_realization_is_allowed",
        )
        self.fixture.product.write_text(stamped, encoding="utf-8")
        self.assertEqual(len(self.fixture.inspect().offenders), 1)

    def test_activation_without_realization_is_caught(self) -> None:
        self.fixture.product.write_text(
            PRODUCT.replace("status: planned", "status: active"), encoding="utf-8"
        )
        self.assertEqual(len(self.fixture.inspect().offenders), 1)

    def test_a_realization_stamp_may_not_smuggle_a_body_edit(self) -> None:
        stamped = (
            PRODUCT.replace("status: planned", "status: active")
            .replace(
                "realized: null",
                "realized: tests/arch/test_product_immutability.py::test_stamping_a_realization_is_allowed",
            )
            .replace("так, а не иначе", "уже совсем иначе")
        )
        self.fixture.product.write_text(stamped, encoding="utf-8")
        self.assertEqual(len(self.fixture.inspect().offenders), 1)

    def test_editing_a_product_rule_without_a_new_ground_is_caught(self) -> None:
        """Silently rewording a rule is silently moving the promise."""
        self.fixture.rule.write_text(
            RULE.replace("так, а не иначе", "уже совсем иначе"), encoding="utf-8"
        )
        verdict = self.fixture.inspect()
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("без нового решения", verdict.offenders[0])

    def test_a_new_invariant_is_not_a_product_ground(self) -> None:
        verdict = self.point_rule_at(
            "invariants/INV.WIRE.WHY-IT-CHANGES.md",
            INVARIANT_GROUND,
            "INV.WIRE.WHY-IT-CHANGES",
        )
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("decision", verdict.offenders[0])

    def test_a_planned_decision_is_not_a_product_ground(self) -> None:
        verdict = self.point_rule_at(
            "decisions/2026-03-03-why-it-changes.md",
            PLANNED_GROUND,
            "DEC.2026-03-03.WHY-IT-CHANGES",
        )
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("planned", verdict.offenders[0])

    def test_a_process_decision_is_not_a_product_ground(self) -> None:
        verdict = self.point_rule_at(
            "decisions/2026-03-03-why-it-changes.md",
            PROCESS_GROUND,
            "DEC.2026-03-03.WHY-IT-CHANGES",
        )
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("process", verdict.offenders[0])

    def test_an_unrealized_active_decision_is_not_a_product_ground(self) -> None:
        verdict = self.point_rule_at(
            "decisions/2026-03-03-why-it-changes.md",
            UNREALIZED_GROUND,
            "DEC.2026-03-03.WHY-IT-CHANGES",
        )
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("realized", verdict.offenders[0])

    def test_an_active_decision_with_missing_evidence_is_not_a_product_ground(self) -> None:
        verdict = self.point_rule_at(
            "decisions/2026-03-03-why-it-changes.md",
            MISSING_EVIDENCE_GROUND,
            "DEC.2026-03-03.WHY-IT-CHANGES",
        )
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("realized", verdict.offenders[0])

    def test_an_active_realized_product_decision_is_a_ground(self) -> None:
        verdict = self.point_rule_at(
            "decisions/2026-03-03-why-it-changes.md",
            GROUND,
            "DEC.2026-03-03.WHY-IT-CHANGES",
        )
        self.assertEqual(verdict.offenders, ())

    def test_an_existing_ground_does_not_cover_a_new_change(self) -> None:
        """A decision written earlier did not foresee today's edit."""
        self.fixture.rule.write_text(
            RULE.replace("так, а не иначе", "уже совсем иначе"), encoding="utf-8"
        )
        verdict = self.fixture.inspect()
        self.assertEqual(len(verdict.offenders), 1)

    def test_editing_a_process_record_is_allowed(self) -> None:
        """A process rule exists to be rebuilt when development gets awkward."""
        self.fixture.process.write_text(
            PROCESS.replace("так, а не иначе", "уже совсем иначе"), encoding="utf-8"
        )
        self.assertEqual(self.fixture.inspect().offenders, ())

    def test_the_side_is_read_from_the_base_not_from_the_edit(self) -> None:
        """Otherwise the rule is escaped by relabelling the record on the way out."""
        self.fixture.product.write_text(
            PRODUCT.replace("governs: product", "governs: process").replace(
                "так, а не иначе", "уже совсем иначе"
            ),
            encoding="utf-8",
        )
        self.assertEqual(len(self.fixture.inspect().offenders), 1)


class LiveTreeTests(unittest.TestCase):
    def test_the_live_tree_holds_no_edited_product_record(self) -> None:
        """Green here is worth only as much as the count it reports.

        `arch/` has not reached `main`, so today this compares nothing. The
        assertion states that plainly instead of letting an empty comparison
        read as a clean one.
        """
        base = subprocess.run(
            ["git", "rev-parse", "--verify", "--quiet", "origin/main"],
            cwd=REPO_ROOT, capture_output=True, text=True,
        )
        if base.returncode != 0:
            self.skipTest("origin/main is not fetched in this checkout")

        verdict = IMMUTABILITY.inspect(REPO_ROOT, "origin/main")
        self.assertEqual(verdict.offenders, ())

        on_base = subprocess.run(
            ["git", "ls-tree", "-r", "--name-only", "origin/main", "--", "arch/"],
            cwd=REPO_ROOT, capture_output=True, text=True, check=True,
        ).stdout.split()
        if not on_base:
            self.assertEqual(
                verdict.compared, 0,
                "no registry on the base branch, so nothing can be compared yet",
            )
        else:
            self.assertGreater(
                verdict.compared, 0,
                "the base branch carries records, so the guard must have compared some",
            )


if __name__ == "__main__":
    unittest.main()
