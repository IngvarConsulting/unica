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
    """A section for a file that already existed."""
    return f"diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}"


def created_diff_for(path: str, body: str) -> str:
    """A section for a file the change creates: the old side is `/dev/null`."""
    return f"diff --git a/{path} b/{path}\n--- /dev/null\n+++ b/{path}\n{body}"


def deleted_diff_for(path: str, body: str) -> str:
    """A section for a file the change deletes: the new side is `/dev/null`."""
    return (
        f"diff --git a/{path} b/{path}\ndeleted file mode 100644\n"
        f"--- a/{path}\n+++ /dev/null\n{body}"
    )


def renamed_diff_for(old: str, new: str) -> str:
    """A section for a file `git mv` moved without editing it.

    Checked against real `git mv` output: a 100% rename carries no `---`, no
    `+++` and no hunk whatsoever -- the two paths appear only here. A parser
    that reads paths from `---`/`+++` alone sees this section as empty, which
    makes `git mv` the quietest way to take a record out of the catalogue.
    """
    return (
        f"diff --git a/{old} b/{new}\nsimilarity index 100%\n"
        f"rename from {old}\nrename to {new}\n"
    )


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
        ) + created_diff_for(
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
        ) + created_diff_for(
            "spec/decisions/0015-form-rename.md", "@@ -0,0 +1 @@\n+# ADR-0015\n"
        )
        original = sys.stdin
        sys.stdin = io.StringIO(diff)
        try:
            self.assertEqual(self.guard.main(["-"]), 0)
        finally:
            sys.stdin = original


class DiffGrammarTests(unittest.TestCase):
    """`---` and `+++` are file headers only where the grammar puts them.

    Hunk content can look exactly like a header. Removing a line whose text is
    `-- spec/architecture/x.md` renders as `--- spec/architecture/x.md`; adding
    one whose text is `++ b/x.md` renders as `+++ b/x.md`. A parser that reads
    those as headers lets a diff claim files it never touched -- and lets a
    single planted line switch off the check for the file that follows.
    """

    def setUp(self) -> None:
        self.guard = load_guard()

    def test_a_removed_content_line_is_not_an_architecture_file(self) -> None:
        """Verified against real `git diff` output, not a hand-written string."""
        diff = diff_for(
            "README.md",
            "@@ -2 +1,0 @@ line one\n--- spec/architecture/invariants.md\n",
        ) + diff_for(
            REGISTRY,
            '@@ -2,0 +3 @@\n+    name: "unica.new.tool",\n',
        )
        change = self.guard.analyze_diff(diff)

        self.assertEqual(change.added, {"unica.new.tool"})
        self.assertEqual(change.architecture_files, set())
        self.assertTrue(change.is_violation)

    def test_an_added_content_line_does_not_open_a_new_file(self) -> None:
        diff = diff_for(
            "spec/decisions/0008-public-marketplace-thin-runtime.md",
            "@@ -1,0 +2 @@\n+++ b/elsewhere.md\n"
            "@@ -3 +4 @@\n-- Дата: `2026-07-19`\n+- Дата: `2026-07-28`\n",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertTrue(
            any("acceptance date rewritten" in violation for violation in violations),
            violations,
        )

    def test_a_planted_dev_null_line_cannot_hide_the_next_file(self) -> None:
        diff = diff_for("README.md", "@@ -2 +1,0 @@\n--- /dev/null\n") + diff_for(
            "spec/decisions/0008-public-marketplace-thin-runtime.md",
            "@@ -3 +3 @@\n-- Дата: `2026-07-19`\n+- Дата: `2026-07-28`\n",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("acceptance date rewritten", violations[0])

    def test_a_rename_without_content_lines_is_ignored(self) -> None:
        diff = (
            "diff --git a/spec/decisions/0008-old.md b/spec/decisions/0008-new.md\n"
            "similarity index 100%\n"
            "rename from spec/decisions/0008-old.md\n"
            "rename to spec/decisions/0008-new.md\n"
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])
        self.assertFalse(self.guard.analyze_diff(diff).touches_public_surface)


class ArchitectureEvidenceTests(unittest.TestCase):
    """Evidence has to be contract-relevant, not any file under `spec/`."""

    def setUp(self) -> None:
        self.guard = load_guard()

    def test_a_cosmetic_glossary_edit_is_not_evidence(self) -> None:
        diff = diff_for(
            REGISTRY,
            '@@ -100,0 +101 @@\n+            name: "unica.new.tool",\n',
        ) + diff_for(
            "spec/architecture/glossary.md",
            "@@ -5 +5 @@\n-Термин — определение.\n+Термин — определение (уточнено).\n",
        )
        change = self.guard.analyze_diff(diff)

        self.assertEqual(change.architecture_files, set())
        self.assertTrue(change.is_violation)

    def test_contract_relevant_documents_are_evidence(self) -> None:
        for evidence in (
            "spec/decisions/0017-form-rename.md",
            "spec/architecture/invariants.md",
            "spec/architecture/quality-requirements.md",
            "spec/acceptance/unica-mcp-validation.md",
        ):
            with self.subTest(evidence=evidence):
                diff = diff_for(
                    REGISTRY,
                    '@@ -100,0 +101 @@\n+            name: "unica.new.tool",\n',
                ) + diff_for(evidence, "@@ -1 +1 @@\n-старое\n+новое\n")

                self.assertFalse(self.guard.analyze_diff(diff).is_violation)

    def test_a_deleted_registry_still_reports_its_removed_tools(self) -> None:
        diff = deleted_diff_for(
            REGISTRY,
            '@@ -1,2 +0,0 @@\n-pub fn tools() {\n-    name: "unica.code.grep",\n',
        )
        change = self.guard.analyze_diff(diff)

        self.assertEqual(change.removed, {"unica.code.grep"})
        self.assertTrue(change.is_violation)


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
            "@@ -20 +20,2 @@\n-старый текст\n+новый текст\n+- Обновлено: `2026-07-29`\n",
        ) + created_diff_for(
            "spec/decisions/0016-new.md",
            "@@ -0,0 +1,2 @@\n+- Статус: `accepted`\n+- Дата: `2026-07-29`\n",
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_a_created_record_alone_is_never_a_rewrite(self) -> None:
        """A diff that only adds a record must not be judged as an edit.

        The old-side header of a created file is `--- /dev/null`; existence is
        decided from that path rather than from the shape of the pattern that
        matched it.
        """
        diff = created_diff_for(
            "spec/decisions/0017-new.md",
            "@@ -0,0 +1,2 @@\n+- Статус: `accepted`\n+- Дата: `2026-07-29`\n",
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_an_unknown_status_is_a_violation(self) -> None:
        """`accepted -> draft` walks out of the catalogue instead of backwards.

        Ranking only known statuses let any word the catalogue does not define
        pass silently, which is the cheapest way to unaccept a record.
        """
        diff = diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "@@ -3 +3 @@\n-- Статус: `accepted`\n+- Статус: `draft`\n",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("unknown status", violations[0])

    def test_deleting_an_accepted_record_is_a_violation(self) -> None:
        """Removal is the most complete rewrite: every reference dangles."""
        diff = deleted_diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "@@ -1,4 +0,0 @@\n-# ADR-0011\n-\n-- Статус: `accepted`\n"
            "-- Дата: `2026-07-21`\n",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("record deleted", violations[0])

    def test_deleting_a_proposed_record_is_allowed(self) -> None:
        """A record that was never binding may be withdrawn."""
        diff = deleted_diff_for(
            "spec/decisions/0018-withdrawn.md",
            "@@ -1,2 +0,0 @@\n-# ADR-0018\n-- Статус: `proposed`\n",
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_moving_a_record_out_of_the_catalogue_is_a_violation(self) -> None:
        """`git mv` removes a record as completely as `git rm`, and more quietly.

        A 100% rename shows no content, so the removed status is invisible and
        the deletion rule never sees a file. The record still leaves
        `spec/decisions/`, and every citation of ADR-0011 stops resolving.
        """
        diff = renamed_diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "docs/attic/0011-canonical-dcs-domain.md",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("left the catalogue", violations[0])

    def test_renumbering_a_record_is_a_violation(self) -> None:
        """The ID is the citation. Moving it is the same defect as deleting it."""
        diff = renamed_diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "spec/decisions/0019-canonical-dcs-domain.md",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("0011", violations[0])

    def test_retitling_a_record_under_the_same_id_is_allowed(self) -> None:
        """Fixing the slug keeps every citation working, so it stays legal."""
        diff = renamed_diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "spec/decisions/0011-canonical-dcs-domain-model.md",
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_moving_a_document_into_the_catalogue_is_not_a_rewrite(self) -> None:
        """Judgement starts from the old side: a non-record states nothing yet."""
        diff = renamed_diff_for(
            "docs/notes/dcs-domain.md",
            "spec/decisions/0020-canonical-dcs-domain.md",
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_dropping_the_status_field_is_a_violation(self) -> None:
        """The cheapest unaccept of all: remove the status instead of lowering it.

        `accepted -> draft` is caught as an unknown status, but a record with no
        `Статус` line at all is outside the catalogue just the same, and the
        rollback rules need two sides to compare.
        """
        diff = diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "@@ -3 +2,0 @@\n-- Статус: `accepted`\n",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("status field removed", violations[0])

    def test_dropping_the_acceptance_date_is_a_violation(self) -> None:
        diff = diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "@@ -4 +3,0 @@\n-- Дата: `2026-07-21`\n",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("acceptance date removed", violations[0])

    def test_translating_a_field_name_keeps_both_sides_and_is_allowed(self) -> None:
        """The field-removal rule must not fire on a renamed field.

        This branch rewrites `Status`/`Date` into `Статус`/`Дата`. Both spellings
        are the same field, so the diff shows a value on each side and nothing
        was dropped.
        """
        diff = diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "@@ -3,2 +3,2 @@\n-- Status: accepted\n-- Date: 2026-07-21\n"
            "+- Статус: `accepted`\n+- Дата: `2026-07-21`\n",
        )

        self.assertEqual(self.guard.analyze_decision_records(diff), [])

    def test_replacing_the_meaning_under_an_unchanged_header_is_a_violation(self) -> None:
        """The header can stay put while the decision underneath is reversed.

        Date and status alone cannot see this. The `Обновлено` stamp can: the
        invariant asks every editorial change to carry it, so prose that moves
        without it is either an unmarked edit or a new decision in disguise.
        """
        diff = diff_for(
            "spec/decisions/0011-canonical-dcs-domain.md",
            "@@ -20 +20 @@\n-Мы выбираем PostgreSQL как хранилище.\n"
            "+Мы выбираем MySQL и отказываемся от прежнего выбора.\n",
        )
        violations = self.guard.analyze_decision_records(diff)

        self.assertEqual(len(violations), 1, violations)
        self.assertIn("without an Updated field", violations[0])


if __name__ == "__main__":
    unittest.main()
