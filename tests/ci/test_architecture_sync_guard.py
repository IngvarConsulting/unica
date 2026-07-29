"""Unit tests for the public-surface synchronization guard.

The guard's value depends entirely on it being precise: it must fail a change
that adds a public tool without describing it, and it must stay silent on
ordinary refactoring. These tests exercise the pure diff classifier on
synthetic input, so they never touch git or the working tree.
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
REGISTRY = "crates/unica-coder/src/application/mod.rs"


def load_guard():
    module_path = REPO_ROOT / "scripts" / "ci" / "check-architecture-sync.py"
    spec = importlib.util.spec_from_file_location("check_architecture_sync", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def diff_for(path: str, body: str) -> str:
    return f"diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}"


def subprocess_result(returncode: int, stdout: str = ""):
    """Minimal stand-in for `subprocess.CompletedProcess`."""

    class Result:
        def __init__(self) -> None:
            self.returncode = returncode
            self.stdout = stdout
            self.stderr = ""

    return Result()


class DiffClassifierTests(unittest.TestCase):
    def setUp(self) -> None:
        self.guard = load_guard()

    def test_added_tool_without_spec_change_is_a_violation(self) -> None:
        diff = diff_for(
            REGISTRY,
            '@@ -100,0 +101,3 @@\n+        ToolSpec {\n+            name: "unica.form.rename",\n+        },\n',
        )
        change = self.guard.analyze_diff(diff)

        self.assertEqual(change.added, {"unica.form.rename"})
        self.assertTrue(change.is_violation)

    def test_added_tool_with_decision_record_is_accepted(self) -> None:
        diff = diff_for(
            REGISTRY,
            '@@ -100,0 +101 @@\n+            name: "unica.form.rename",\n',
        ) + diff_for(
            "spec/decisions/0015-form-rename.md",
            "@@ -0,0 +1 @@\n+# ADR-0015\n",
        )
        change = self.guard.analyze_diff(diff)

        self.assertEqual(change.added, {"unica.form.rename"})
        self.assertTrue(change.touches_architecture)
        self.assertFalse(change.is_violation)

    def test_removed_tool_without_spec_change_is_a_violation(self) -> None:
        diff = diff_for(
            REGISTRY,
            '@@ -100 +99,0 @@\n-            name: "unica.code.grep",\n',
        )
        change = self.guard.analyze_diff(diff)

        self.assertEqual(change.removed, {"unica.code.grep"})
        self.assertTrue(change.is_violation)

    def test_acceptance_or_architecture_change_also_counts_as_evidence(self) -> None:
        for evidence in (
            "spec/acceptance/unica-mcp-validation.md",
            "spec/architecture/invariants.md",
        ):
            with self.subTest(evidence=evidence):
                diff = diff_for(
                    REGISTRY,
                    '@@ -100,0 +101 @@\n+            name: "unica.meta.rename",\n',
                ) + diff_for(evidence, "@@ -1 +1 @@\n-old\n+new\n")
                change = self.guard.analyze_diff(diff)

                self.assertFalse(change.is_violation)

    def test_refactoring_inside_the_registry_does_not_trip_the_guard(self) -> None:
        diff = diff_for(
            REGISTRY,
            "@@ -200,2 +200,2 @@\n"
            "-        let handler = build_handler(context);\n"
            "+        let handler = build_handler(&context);\n",
        )
        change = self.guard.analyze_diff(diff)

        self.assertFalse(change.touches_public_surface)
        self.assertFalse(change.is_violation)

    def test_a_moved_declaration_is_not_a_surface_change(self) -> None:
        diff = diff_for(
            REGISTRY,
            '@@ -100,2 +100,2 @@\n'
            '-            name: "unica.form.edit",\n'
            '+            name: "unica.form.edit",\n',
        )
        change = self.guard.analyze_diff(diff)

        self.assertEqual(change.added, set())
        self.assertEqual(change.removed, set())
        self.assertFalse(change.is_violation)

    def test_a_rename_reports_both_sides(self) -> None:
        diff = diff_for(
            REGISTRY,
            '@@ -100 +100 @@\n'
            '-            name: "unica.code.grep",\n'
            '+            name: "unica.code.search",\n',
        )
        change = self.guard.analyze_diff(diff)

        self.assertEqual(change.added, {"unica.code.search"})
        self.assertEqual(change.removed, {"unica.code.grep"})
        self.assertTrue(change.is_violation)

    def test_tool_names_outside_the_registry_are_ignored(self) -> None:
        diff = diff_for(
            "crates/unica-coder/src/interfaces/mcp.rs",
            '@@ -10,0 +11 @@\n+    if spec.name == "unica.code.patch" {\n',
        )
        change = self.guard.analyze_diff(diff)

        self.assertFalse(change.touches_public_surface)

    def test_empty_diff_is_clean(self) -> None:
        change = self.guard.analyze_diff("")

        self.assertFalse(change.touches_public_surface)
        self.assertFalse(change.is_violation)


class CommandLineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.guard = load_guard()

    def test_explicit_base_wins_over_discovery(self) -> None:
        self.assertEqual(self.guard.resolve_base("release/1.2"), "release/1.2")

    def test_unresolvable_base_skips_instead_of_failing(self) -> None:
        from unittest.mock import patch

        failed = subprocess_result(returncode=1)
        with patch.dict(self.guard.os.environ, {}, clear=True), patch.object(
            self.guard.subprocess, "run", return_value=failed
        ):
            self.assertIsNone(self.guard.resolve_base(None))
            self.assertEqual(self.guard.main([]), 0)

    def test_strict_mode_fails_when_the_base_cannot_be_resolved(self) -> None:
        """A guard that cannot run must say so rather than pass.

        In CI an unusable base means the job is misconfigured. Reporting success
        there is how a guard becomes decorative.
        """
        from unittest.mock import patch

        failed = subprocess_result(returncode=1)
        with patch.dict(self.guard.os.environ, {}, clear=True), patch.object(
            self.guard.subprocess, "run", return_value=failed
        ):
            self.assertEqual(self.guard.main(["--strict"]), 2)

    def test_strict_mode_fails_when_the_diff_cannot_be_read(self) -> None:
        from unittest.mock import patch

        with patch.object(self.guard, "resolve_base", return_value="origin/main"), patch.object(
            self.guard, "read_diff", return_value=None
        ):
            self.assertEqual(self.guard.main(["--strict"]), 2)
            self.assertEqual(self.guard.main([]), 0)

    def test_strict_mode_still_passes_a_clean_diff(self) -> None:
        from unittest.mock import patch

        clean = diff_for("README.md", "@@ -1 +1 @@\n-old\n+new\n")
        with patch.object(self.guard, "resolve_base", return_value="origin/main"), patch.object(
            self.guard, "read_diff", return_value=clean
        ):
            self.assertEqual(self.guard.main(["--strict"]), 0)

    def test_violating_diff_from_stdin_exits_one(self) -> None:
        import io
        import sys

        diff = diff_for(
            REGISTRY,
            '@@ -100,0 +101 @@\n+            name: "unica.form.rename",\n',
        )
        original = sys.stdin
        sys.stdin = io.StringIO(diff)
        try:
            self.assertEqual(self.guard.main(["-"]), 1)
        finally:
            sys.stdin = original

    def test_described_diff_from_stdin_exits_zero(self) -> None:
        import io
        import sys

        diff = diff_for(
            REGISTRY,
            '@@ -100,0 +101 @@\n+            name: "unica.form.rename",\n',
        ) + diff_for("spec/decisions/0015-form-rename.md", "@@ -0,0 +1 @@\n+# ADR-0015\n")
        original = sys.stdin
        sys.stdin = io.StringIO(diff)
        try:
            self.assertEqual(self.guard.main(["-"]), 0)
        finally:
            sys.stdin = original


if __name__ == "__main__":
    unittest.main()


class DecisionRecordImmutabilityTests(unittest.TestCase):
    """INV-DOC-SUPERSEDE-NOT-EDIT: an accepted record is superseded, never rewritten."""

    def setUp(self) -> None:
        self.guard = load_guard()

    def test_moving_the_acceptance_date_is_a_violation(self) -> None:
        diff = diff_for(
            "spec/decisions/0008-public-marketplace-thin-runtime.md",
            "@@ -3 +3 @@\n-- Дата: `2026-07-19`\n+- Дата: `2026-07-28`\n",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("acceptance date rewritten", violations[0])

    def test_walking_the_status_backwards_is_a_violation(self) -> None:
        diff = diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "@@ -3 +3 @@\n-- Статус: `accepted`\n+- Статус: `proposed`\n",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("status moved backwards", violations[0])

    def test_superseding_a_record_is_allowed(self) -> None:
        diff = diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "@@ -3 +3 @@\n-- Статус: `accepted`\n+- Статус: `superseded`\n",
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_editorial_changes_are_allowed(self) -> None:
        """Translations and typo fixes keep the date and the status."""
        diff = diff_for(
            "spec/decisions/0009-os-specific-code-behind-platform-facade.md",
            "@@ -20,2 +20,3 @@\n-OS-specific code lives behind facades.\n"
            "+Зависящий от ОС код живёт за фасадами.\n"
            "+- Обновлено: `2026-07-28`\n",
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_a_brand_new_record_may_say_anything(self) -> None:
        diff = (
            "diff --git a/spec/decisions/0014-new.md b/spec/decisions/0014-new.md\n"
            "--- /dev/null\n+++ b/spec/decisions/0014-new.md\n"
            "@@ -0,0 +1,2 @@\n+- Статус: `accepted`\n+- Дата: `2026-07-28`\n"
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_edits_outside_the_decisions_directory_are_ignored(self) -> None:
        diff = diff_for(
            "spec/architecture/invariants.md",
            "@@ -3 +3 @@\n-- Дата: `2026-07-19`\n+- Дата: `2026-07-28`\n",
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_a_rewritten_record_fails_the_command(self) -> None:
        import io
        import sys

        diff = diff_for(
            "spec/decisions/0008-public-marketplace-thin-runtime.md",
            "@@ -3 +3 @@\n-- Дата: `2026-07-19`\n+- Дата: `2026-07-28`\n",
        )
        original = sys.stdin
        sys.stdin = io.StringIO(diff)
        try:
            self.assertEqual(self.guard.main(["-"]), 1)
        finally:
            sys.stdin = original

    def test_an_edited_record_followed_by_a_new_one_is_still_judged(self) -> None:
        """State from one file must not decide the verdict for another.

        A unified diff marks a pre-existing file with `--- a/<path>` and a
        created one with `--- /dev/null`. If the flag that records "this file
        existed" outlived its file, an edit followed by an addition would be
        flushed under the next file's flag and its violation would vanish.
        """
        diff = diff_for(
            "spec/decisions/0008-public-marketplace-thin-runtime.md",
            "@@ -3 +3 @@\n-- Дата: `2026-07-19`\n+- Дата: `2026-07-28`\n",
        ) + (
            "diff --git a/spec/decisions/0016-new.md b/spec/decisions/0016-new.md\n"
            "--- /dev/null\n+++ b/spec/decisions/0016-new.md\n"
            "@@ -0,0 +1 @@\n+- Дата: `2026-07-29`\n"
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("0008-public-marketplace-thin-runtime.md", violations[0])

    def test_a_new_record_after_an_edited_one_is_not_flagged(self) -> None:
        """The reverse direction: a created file must not inherit `existed`."""
        diff = diff_for(
            "spec/decisions/0008-public-marketplace-thin-runtime.md",
            "@@ -20 +20 @@\n-старый текст\n+новый текст\n",
        ) + (
            "diff --git a/spec/decisions/0016-new.md b/spec/decisions/0016-new.md\n"
            "--- /dev/null\n+++ b/spec/decisions/0016-new.md\n"
            "@@ -0,0 +1,2 @@\n+- Статус: `accepted`\n+- Дата: `2026-07-29`\n"
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])
