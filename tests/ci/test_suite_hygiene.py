"""Structural checks on the test suite itself.

CI runs the suites through `unittest discover`, but a developer debugging one
file runs that file directly, and the two must collect the same tests. When
they do not, the direct run is quietly a subset: the file looks green while a
whole class never executes. That happened here -- `unittest.main()` sat above
the last class in `test_architecture_sync_guard.py`, so running the file
directly collected 16 tests where discovery collected 26, and the ten missing
ones were the immutability suite for accepted decision records.
"""

from __future__ import annotations

import ast
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
# Every suite in the tree, run or parked: the rule is about how a file
# collects when someone runs it directly, which does not depend on CI.
SUITE_ROOTS = tuple(
    REPO_ROOT / "tests" / name for name in ("ci", "arch", "parity", "harness")
)


def suite_files() -> list[Path]:
    """Every file `unittest discover` collects from the suites CI runs."""
    files = []
    for root in SUITE_ROOTS:
        files.extend(sorted(root.glob("test*.py")))
    return files


def main_guard_line(tree: ast.Module) -> int | None:
    """Line of the top-level `if __name__ == "__main__":` block, if any."""
    for node in tree.body:
        if not isinstance(node, ast.If):
            continue
        test = node.test
        if (
            isinstance(test, ast.Compare)
            and isinstance(test.left, ast.Name)
            and test.left.id == "__name__"
        ):
            return node.lineno
    return None


class SuiteCollectionTests(unittest.TestCase):
    def test_the_suites_are_not_empty(self) -> None:
        self.assertGreater(len(suite_files()), 20, "no suite files were found")

    def test_nothing_is_defined_after_the_main_guard(self) -> None:
        """A direct run must collect the same tests as discovery.

        `unittest.main()` collects the classes defined *before* it runs.
        Anything defined after the guard exists for discovery and is invisible
        to `python tests/ci/<file>.py`, which is the run a developer trusts
        while debugging.
        """
        offenders = []
        for path in suite_files():
            tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
            guard = main_guard_line(tree)
            if guard is None:
                continue
            stranded = [
                node.name
                for node in tree.body
                if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef))
                and node.lineno > guard
            ]
            if stranded:
                offenders.append(
                    f"{path.relative_to(REPO_ROOT).as_posix()}: "
                    f"{', '.join(stranded)} defined after `unittest.main()`"
                )
        self.assertEqual(
            offenders,
            [],
            "move the `__main__` guard to the end of the file so a direct run "
            "and `unittest discover` collect the same tests",
        )


if __name__ == "__main__":
    unittest.main()
