"""Contract tests for design documents and plans.

A design document records how a choice was reached. It is not normative, and it
is not a substitute for a decision record. The header enforced here exists to
make one question impossible to skip: did this design change an architectural
contract, and if so, which record carries that change?

`Decision: none` is a legitimate answer for most designs. It is also a claim a
reviewer can reject, which is the point — the question gets answered in the
artifact instead of being forgotten.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

DESIGN_DIR = REPO_ROOT / "docs" / "design"
PLANS_DIR = REPO_ROOT / "docs" / "plans"
DECISIONS_DIR = REPO_ROOT / "spec" / "decisions"
SPEC_DIR = REPO_ROOT / "spec"

DATE_FIELD = re.compile(r"^- Date: `(?P<date>\d{4}-\d{2}-\d{2})`$", re.MULTILINE)
STATUS_FIELD = re.compile(
    r"^- Status: `(?P<status>draft|approved|superseded)`$", re.MULTILINE
)
DECISION_FIELD = re.compile(
    r"^- Decision: (?:`(?P<adr>ADR-\d{4})`|`none` — .+|none — .+)$", re.MULTILINE
)
ADR_IN_DECISION = re.compile(r"ADR-(?P<number>\d{4})")
ADR_FILE = re.compile(r"^(?P<number>\d{4})-.+\.md$")

ARCHIVE_MARKER = "Архивный материал планирования, а не источник истины"

# The header requirement starts with documents written after the rule landed.
# Rewriting the header of an older document would mean inventing an answer to a
# question nobody asked at the time; those are listed here instead, and the list
# may only shrink.
HEADERLESS_LEGACY = {
    "2026-07-20-safe-single-file-publisher-design.md",
    "2026-07-21-code-patch-v1-safe-design.md",
    "2026-07-22-issue-115-attributions-design.md",
    "2026-07-22-local-1ci-guides-design.md",
    "2026-07-22-pr-182-form-design-guide-fixes-design.md",
    "2026-07-23-issue-185-rlm-stale-content-recovery-design.md",
    "2026-07-23-platform-8-3-27-format-research.md",
    "2026-07-23-platform-8-3-27-format-2-20-design.md",
    "2026-07-23-pr-184-language-aware-synonym-validation-design.md",
    "2026-07-24-effective-compatibility-version-design.md",
    "2026-07-24-updatable-donor-parity-relations-design.md",
    "2026-07-26-diagnostics-path-containment-design.md",
    "2026-07-26-writer-text-snapshot-eol-policy-design.md",
}

# These documents carry the three fields, but put their H1 before them. They
# predate enforcement of the literal "opens with" order in AGENTS.md. Keep the
# debt explicit without making an unrelated PR rewrite archived design history.
TITLE_FIRST_LEGACY = {
    "2026-07-27-provider-neutral-code-intelligence-design.md",
    "2026-07-27-rlm-integration-api-issue-draft.md",
    "2026-07-27-worktree-scoped-provider-state-design.md",
    "2026-07-29-logical-source-addressing-design.md",
    "2026-07-30-bounded-source-resource-access-design.md",
    "2026-07-30-current-source-bsl-outline-design.md",
    "2026-07-30-structured-bsl-outline-result-design.md",
    "2026-07-30-windows-applied-full-dump-design.md",
    "2026-08-02-xdto-package-domain-review-fixes-design.md",
    "2026-08-03-issue-286-rlm-source-generation-design.md",
    "2026-08-03-meta-surface-design.md",
    "2026-08-08-meta-object-integrity-design.md",
    "2026-08-08-platform-help-documentation-provider-design.md",
}


def decision_numbers_on_disk() -> set[str]:
    numbers = set()
    for path in DECISIONS_DIR.glob("*.md"):
        match = ADR_FILE.match(path.name)
        if match:
            numbers.add(match.group("number"))
    return numbers


def design_documents() -> list[Path]:
    return [p for p in sorted(DESIGN_DIR.glob("*.md")) if p.name != "README.md"]


class LayoutTests(unittest.TestCase):
    def test_both_archive_trees_exist_and_are_marked(self) -> None:
        offenders = []
        for directory in (DESIGN_DIR, PLANS_DIR):
            readme = directory / "README.md"
            if not readme.is_file():
                offenders.append(f"{directory.name}: missing README.md")
                continue
            if ARCHIVE_MARKER not in readme.read_text(encoding="utf-8"):
                offenders.append(f"{directory.name}: missing archive marker")
        self.assertEqual(offenders, [])

    def test_retired_locations_are_not_recreated(self) -> None:
        """One location per artifact class.

        These trees held design documents and plans before the layout was
        consolidated. Recreating one splits the same artifact class across two
        places again, which is how four locations accumulated the first time.
        """
        retired = [
            REPO_ROOT / "docs" / "superpowers",
            SPEC_DIR / "designs",
            SPEC_DIR / "plans",
        ]
        existing = [
            path.relative_to(REPO_ROOT).as_posix() for path in retired if path.exists()
        ]
        self.assertEqual(existing, [])

    def test_session_scratch_is_never_tracked(self) -> None:
        """`.superpowers/` is ignored on purpose; `git add -f` defeats that."""
        tracked = sorted((REPO_ROOT / ".superpowers").rglob("*")) if (
            REPO_ROOT / ".superpowers"
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


class DesignHeaderTests(unittest.TestCase):
    def test_required_header_is_the_first_content_in_each_document(self) -> None:
        offenders = []
        required = (
            ("Date", DATE_FIELD),
            ("Status", STATUS_FIELD),
            ("Decision", DECISION_FIELD),
        )
        for path in design_documents():
            if path.name in HEADERLESS_LEGACY or path.name in TITLE_FIRST_LEGACY:
                continue
            lines = [
                line
                for line in path.read_text(encoding="utf-8").splitlines()
                if line.strip()
            ]
            for index, (name, pattern) in enumerate(required):
                if index >= len(lines) or pattern.fullmatch(lines[index]) is None:
                    actual = lines[index] if index < len(lines) else "<missing>"
                    offenders.append(
                        f"{path.name}: content line {index + 1} must be {name}, got {actual!r}"
                    )
        self.assertEqual(
            offenders,
            [],
            "design documents open with Date, Status, Decision; the title follows",
        )

    def test_design_documents_carry_the_required_header(self) -> None:
        offenders = []
        for path in design_documents():
            if path.name in HEADERLESS_LEGACY:
                continue
            text = path.read_text(encoding="utf-8")
            for name, pattern in (
                ("Date", DATE_FIELD),
                ("Status", STATUS_FIELD),
                ("Decision", DECISION_FIELD),
            ):
                if not pattern.search(text):
                    offenders.append(f"{path.name}: missing or malformed {name}")
        self.assertEqual(
            offenders,
            [],
            "see the design-document header contract in AGENTS.md",
        )

    def test_declared_decisions_exist(self) -> None:
        available = decision_numbers_on_disk()
        offenders = []
        for path in design_documents():
            match = DECISION_FIELD.search(path.read_text(encoding="utf-8"))
            if not match or not match.group("adr"):
                continue
            number = ADR_IN_DECISION.search(match.group("adr")).group("number")
            if number not in available:
                offenders.append(f"{path.name}: {match.group('adr')} has no record")
        self.assertEqual(offenders, [])

    def test_legacy_exemption_list_only_names_documents_that_exist(self) -> None:
        """The exemption list may only shrink.

        A stale entry would silently exempt a future document that happens to
        reuse the name.
        """
        present = {path.name for path in design_documents()}
        stale = sorted((HEADERLESS_LEGACY | TITLE_FIRST_LEGACY) - present)
        self.assertEqual(
            stale,
            [],
            "remove exempted names that no longer exist instead of carrying them",
        )
        self.assertEqual(
            HEADERLESS_LEGACY & TITLE_FIRST_LEGACY,
            set(),
            "a design cannot be both headerless and title-first legacy",
        )


class NormativeBoundaryTests(unittest.TestCase):
    def test_no_design_document_claims_to_be_normative(self) -> None:
        """Archived material must not present itself as a rule.

        The words below are how a design document starts pretending to be a
        contract. If a design really needs to state a rule, that rule belongs in
        a decision record or the invariant registry.
        """
        claims = ("INV-", "REQ-")
        offenders = []
        for path in design_documents():
            for number, line in enumerate(
                path.read_text(encoding="utf-8").splitlines(), start=1
            ):
                stripped = line.strip()
                if not stripped.startswith("### "):
                    continue
                if any(claim in stripped for claim in claims):
                    offenders.append(f"{path.name}:{number}: declares {stripped!r}")
        self.assertEqual(
            offenders,
            [],
            "registry entries are declared in spec/, never in a design document",
        )


class LogicalSourceDesignContractTests(unittest.TestCase):
    def test_source_designs_are_approved_and_do_not_restate_stale_wire_shapes(
        self,
    ) -> None:
        logical = (
            DESIGN_DIR / "2026-07-29-logical-source-addressing-design.md"
        ).read_text(encoding="utf-8")
        resources = (
            DESIGN_DIR / "2026-07-30-bounded-source-resource-access-design.md"
        ).read_text(encoding="utf-8")

        self.assertIn("- Status: `approved`", logical)
        self.assertIn("- Status: `approved`", resources)
        self.assertIn("- Decision: `ADR-0021`", logical)
        self.assertIn("- Decision: `ADR-0022`", resources)
        self.assertNotIn("  owner,\n  role,", resources)
        self.assertNotIn('"nextCursor": null', resources)


if __name__ == "__main__":
    unittest.main()
