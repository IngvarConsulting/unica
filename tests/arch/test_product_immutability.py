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
status: active
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
        self._git("add", "arch")
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

    def test_an_untouched_tree_is_clean_and_says_what_it_compared(self) -> None:
        verdict = self.fixture.inspect()
        self.assertEqual(verdict.offenders, ())
        self.assertEqual(verdict.compared, 1, "the process record must not be counted")

    def test_editing_an_accepted_product_record_is_caught(self) -> None:
        self.fixture.product.write_text(
            PRODUCT.replace("так, а не иначе", "уже совсем иначе"), encoding="utf-8"
        )
        verdict = self.fixture.inspect()
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("отредактирована", verdict.offenders[0])

    def test_deleting_an_accepted_product_record_is_caught(self) -> None:
        self.fixture.product.unlink()
        verdict = self.fixture.inspect()
        self.assertEqual(len(verdict.offenders), 1)
        self.assertIn("удалена", verdict.offenders[0])

    def test_stamping_a_supersession_is_allowed(self) -> None:
        """Replacement is the one legitimate edit, and it touches two fields."""
        stamped = PRODUCT.replace("status: active", "status: superseded").replace(
            "superseded-by: null", "superseded-by: DEC.2026-02-02.BETTER-PROMISE"
        )
        self.fixture.product.write_text(stamped, encoding="utf-8")
        self.assertEqual(self.fixture.inspect().offenders, ())

    def test_a_supersession_stamp_may_not_smuggle_a_body_edit(self) -> None:
        stamped = (
            PRODUCT.replace("status: active", "status: superseded")
            .replace("superseded-by: null", "superseded-by: DEC.2026-02-02.BETTER-PROMISE")
            .replace("так, а не иначе", "уже совсем иначе")
        )
        self.fixture.product.write_text(stamped, encoding="utf-8")
        self.assertEqual(len(self.fixture.inspect().offenders), 1)

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
