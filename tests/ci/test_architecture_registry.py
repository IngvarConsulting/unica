"""Contract tests for the machine-checkable architecture registry.

The registry lives in two documents that share one record format:

* `spec/architecture/invariants.md` holds `INV-*` records: rules that must not
  break silently.
* `spec/architecture/quality-requirements.md` holds `REQ-*` records: measurable
  quality scenarios.

These tests keep the registry honest. A record that names a check which does not
exist, an index that drifted from the files it indexes, or a link that no longer
resolves is a defect in the architecture layer, not a formatting nit.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

ARCHITECTURE_DIR = REPO_ROOT / "spec" / "architecture"
INVARIANTS = ARCHITECTURE_DIR / "invariants.md"
REQUIREMENTS = ARCHITECTURE_DIR / "quality-requirements.md"
DECISIONS_DIR = REPO_ROOT / "spec" / "decisions"
DECISIONS_INDEX = DECISIONS_DIR / "README.md"
SPEC_INDEX = REPO_ROOT / "spec" / "README.md"

RECORD_HEADING = re.compile(r"^### (?P<id>\S+) — (?P<name>.+)$")
# `INV-<ОБЛАСТЬ>-<КОД>`: код смысловой, без цифр, потому что реестр — множество,
# а не последовательность, и номер сообщал бы несуществующий приоритет.
RECORD_ID = re.compile(r"^(?:INV|REQ)-[A-Z]+(?:-[A-Z]+)+$")
MAX_ID_LENGTH = 40
FIELD_START = re.compile(r"^- \*\*(?P<field>Rule|Decision|Check|Scope):\*\* (?P<value>.*)$")
CHECK_HEAD = re.compile(r"^`(?P<cls>[a-z][a-z-]*)` — (?P<target>.+)$", re.DOTALL)
BACKTICKED = re.compile(r"^`(?P<target>[^`]+)`$")
ADR_REFERENCE = re.compile(r"ADR-(?P<number>[0-9]{4})")
ADR_FILE = re.compile(r"^(?P<number>[0-9]{4})-.+\.md$")
MARKDOWN_LINK = re.compile(r"\[[^\]]*\]\((?P<target>[^)\s]+)\)")

CHECK_CLASSES = {"ci-test", "guard-script", "doc-assert", "release-gate", "manual"}
SCOPES = {"source", "packaged", "ci", "release", "runtime"}

# Ratchet: the number of registry records whose only evidence is a human
# re-reading the code. This constant may be lowered when a manual check is
# automated. It must never be raised: a new rule without an automated check is
# a decision to accept documentation drift, and it belongs in a risk record
# (spec/architecture/risks.md), not here.
MAX_MANUAL_CHECKS = 5

# An architecture document may cite a decision record to explain a choice.
# Linking most of the catalogue turns the document into a second decision index,
# which is how the decisions chapter drifted five records behind before the
# registry rework -- and how the same list then reappeared in another chapter.
MAX_DECISION_LINKS_PER_DOCUMENT = 3

# Registry fields carry normative text, and normative text is Russian
# (INV-DOC-RUSSIAN-NORMATIVE). Identifiers stay Latin, so a rule may legitimately be almost all
# backticked names; only the prose around them is checked.
CYRILLIC = re.compile(r"[Ѐ-ӿ]")
BACKTICKED_SPAN = re.compile(r"`[^`]*`")
NORMATIVE_FIELDS = ("Rule",)

# Formulations that describe Unica as a single-host product. ADR-0012 made the
# plugin directory serve both Codex and Claude Code, so these are wrong in the
# active layer. Historical records keep their original text.
SINGLE_HOST_PHRASES = (
    "Codex plugin",
    "Codex-плагин",
    "плагин Codex",
    "fresh Codex visibility",
    "Codex operation instruction",
)

ARCHIVE_MARKER = "Архивный материал планирования, а не источник истины"
ARCHIVE_INDEXES = (
    REPO_ROOT / "docs" / "design" / "README.md",
    REPO_ROOT / "docs" / "plans" / "README.md",
)


class Record:
    def __init__(self, identifier: str, name: str, source: Path, line: int) -> None:
        self.id = identifier
        self.name = name
        self.source = source
        self.line = line
        self.fields: dict[str, list[str]] = {}

    @property
    def where(self) -> str:
        return f"{self.source.relative_to(REPO_ROOT).as_posix()}:{self.line} ({self.id})"

    def add(self, field: str, value: str) -> None:
        self.fields.setdefault(field, []).append(value)

    def one(self, field: str) -> str | None:
        values = self.fields.get(field)
        return values[0] if values else None


def parse_records(path: Path) -> list[Record]:
    """Parse registry records, joining indented continuation lines."""
    records: list[Record] = []
    current: Record | None = None
    field: str | None = None
    buffer: list[str] = []

    def flush() -> None:
        nonlocal field, buffer
        if current is not None and field is not None:
            current.add(field, " ".join(part.strip() for part in buffer).strip())
        field = None
        buffer = []

    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        heading = RECORD_HEADING.match(raw)
        if heading:
            flush()
            current = Record(heading.group("id"), heading.group("name"), path, number)
            records.append(current)
            continue

        start = FIELD_START.match(raw)
        if start:
            flush()
            field = start.group("field")
            buffer = [start.group("value")]
            continue

        if field is not None:
            if raw.startswith("  ") and raw.strip():
                buffer.append(raw)
                continue
            flush()

    flush()
    return records


def all_records() -> list[Record]:
    return parse_records(INVARIANTS) + parse_records(REQUIREMENTS)


def decision_numbers_on_disk() -> set[str]:
    numbers = set()
    for path in DECISIONS_DIR.glob("*.md"):
        match = ADR_FILE.match(path.name)
        if match:
            numbers.add(match.group("number"))
    return numbers


class RegistryFormatTests(unittest.TestCase):
    def setUp(self) -> None:
        self.records = all_records()

    def test_registry_is_not_empty(self) -> None:
        self.assertGreater(len(self.records), 20, "registry parsed as nearly empty")

    def test_every_heading_is_a_valid_record_id(self) -> None:
        offenders = [
            record.where for record in self.records if not RECORD_ID.match(record.id)
        ]
        self.assertEqual(offenders, [], "level-3 headings in the registry must be records")

    def test_record_ids_carry_meaning_not_a_number(self) -> None:
        """Коды смысловые: цифра в идентификаторе вернула бы ложный приоритет."""
        offenders = [
            record.where
            for record in self.records
            if any(char.isdigit() for char in record.id) or len(record.id) > MAX_ID_LENGTH
        ]
        self.assertEqual(offenders, [])

    def test_record_ids_are_unique(self) -> None:
        seen: dict[str, str] = {}
        duplicates = []
        for record in self.records:
            if record.id in seen:
                duplicates.append(f"{record.where} duplicates {seen[record.id]}")
            seen[record.id] = record.where
        self.assertEqual(duplicates, [])

    def test_every_record_has_the_required_fields(self) -> None:
        offenders = []
        for record in self.records:
            for field in ("Rule", "Decision", "Scope"):
                if not record.one(field):
                    offenders.append(f"{record.where}: missing {field}")
            if not record.fields.get("Check"):
                offenders.append(f"{record.where}: missing Check")
        self.assertEqual(offenders, [])

    def test_scopes_are_known(self) -> None:
        offenders = []
        for record in self.records:
            value = record.one("Scope") or ""
            for scope in (item.strip() for item in value.split(",")):
                if scope and scope not in SCOPES:
                    offenders.append(f"{record.where}: unknown scope {scope!r}")
        self.assertEqual(offenders, [])

    def test_rules_are_single_statements(self) -> None:
        offenders = [
            record.where
            for record in self.records
            if len(record.fields.get("Rule", [])) != 1
        ]
        self.assertEqual(offenders, [], "a record states exactly one rule")

    def test_normative_fields_are_written_in_russian(self) -> None:
        """Rules are stated in Russian; only identifiers stay Latin.

        Backticked spans are dropped before the check, because a rule may
        legitimately consist mostly of tool names and paths. What is left is the
        prose that carries the meaning, and that prose must be in one language.
        """
        offenders = []
        for record in self.records:
            for field in NORMATIVE_FIELDS:
                for value in record.fields.get(field, []):
                    prose = BACKTICKED_SPAN.sub(" ", value).strip()
                    if not prose:
                        continue
                    if not CYRILLIC.search(prose):
                        offenders.append(f"{record.where}: {field} is not Russian")
        self.assertEqual(
            offenders,
            [],
            "normative text is Russian so one grep finds every statement of a rule",
        )


class IdentifierLedgerTests(unittest.TestCase):
    """Identifiers are unique corpus-wide and are never handed to a new rule."""

    def declared_ids(self) -> dict[str, list[str]]:
        """Every place the corpus declares a record, keyed by identifier.

        A declaration is a level-3 heading, which is how the registry defines a
        record. A mention inside prose is a reference, not a declaration.
        """
        declarations: dict[str, list[str]] = {}
        roots = [REPO_ROOT / "spec", REPO_ROOT / "docs"]
        for root in roots:
            for path in sorted(root.rglob("*.md")):
                for number, line in enumerate(
                    path.read_text(encoding="utf-8").splitlines(), start=1
                ):
                    heading = RECORD_HEADING.match(line)
                    if heading and RECORD_ID.match(heading.group("id")):
                        where = f"{path.relative_to(REPO_ROOT).as_posix()}:{number}"
                        declarations.setdefault(heading.group("id"), []).append(where)
        return declarations

    def test_identifiers_are_declared_once_in_the_whole_corpus(self) -> None:
        offenders = [
            f"{identifier} declared at {', '.join(places)}"
            for identifier, places in sorted(self.declared_ids().items())
            if len(places) > 1
        ]
        self.assertEqual(offenders, [])

    def test_retired_identifiers_are_not_reissued(self) -> None:
        """A retired identifier stays retired.

        The registry records withdrawn identifiers under a dedicated heading so
        that a future rule cannot quietly inherit the meaning of a deleted one.
        """
        text = INVARIANTS.read_text(encoding="utf-8")
        section = re.search(
            r"## Выведенные из обращения идентификаторы\n(?P<body>.*?)(?=\n## |\Z)",
            text,
            re.DOTALL,
        )
        self.assertIsNotNone(
            section, "invariants.md must carry a retired-identifier ledger"
        )
        retired = set(
            re.findall(
                r"\b((?:INV|REQ)-[A-Z]+(?:-[A-Z]+)+)\b", section.group("body")
            )
        )
        active = set(self.declared_ids())
        reissued = sorted(retired & active)
        self.assertEqual(
            reissued, [], "a retired identifier may never be given to a new rule"
        )


class RegistryCheckTests(unittest.TestCase):
    def setUp(self) -> None:
        self.records = all_records()

    def parsed_checks(self) -> list[tuple[Record, str, str]]:
        parsed = []
        for record in self.records:
            for value in record.fields.get("Check", []):
                match = CHECK_HEAD.match(value)
                if match:
                    parsed.append((record, match.group("cls"), match.group("target")))
        return parsed

    def test_check_lines_are_well_formed(self) -> None:
        offenders = []
        for record in self.records:
            for value in record.fields.get("Check", []):
                if not CHECK_HEAD.match(value):
                    offenders.append(f"{record.where}: unparsable check {value!r}")
        self.assertEqual(offenders, [])

    def test_check_classes_are_known(self) -> None:
        offenders = [
            f"{record.where}: unknown check class {cls!r}"
            for record, cls, _ in self.parsed_checks()
            if cls not in CHECK_CLASSES
        ]
        self.assertEqual(offenders, [])

    def test_automated_checks_name_a_file_that_exists(self) -> None:
        offenders = []
        for record, cls, target in self.parsed_checks():
            if cls == "manual":
                continue
            backticked = BACKTICKED.match(target.strip())
            if not backticked:
                offenders.append(
                    f"{record.where}: {cls} target must be one backticked path, got {target!r}"
                )
                continue
            path = REPO_ROOT / backticked.group("target")
            if not path.exists():
                offenders.append(f"{record.where}: missing check target {backticked.group('target')}")
        self.assertEqual(offenders, [], "a registry entry may not point at a check that does not exist")

    def test_manual_checks_stay_within_budget(self) -> None:
        manual = [
            record.where for record, cls, _ in self.parsed_checks() if cls == "manual"
        ]
        self.assertLessEqual(
            len(manual),
            MAX_MANUAL_CHECKS,
            "manual checks may only be traded for automation, never added:\n"
            + "\n".join(manual),
        )

    def test_referenced_decisions_exist(self) -> None:
        available = decision_numbers_on_disk()
        offenders = []
        for record in self.records:
            value = record.one("Decision") or ""
            if value.strip() == "n/a":
                continue
            referenced = ADR_REFERENCE.findall(value)
            if not referenced:
                offenders.append(f"{record.where}: Decision must name an ADR or be 'n/a'")
                continue
            for number in referenced:
                if number not in available:
                    offenders.append(f"{record.where}: ADR-{number} has no file in spec/decisions")
        self.assertEqual(offenders, [])


class IndexSynchronizationTests(unittest.TestCase):
    def test_decision_index_lists_exactly_the_records_on_disk(self) -> None:
        index_text = DECISIONS_INDEX.read_text(encoding="utf-8")
        linked = {
            match.group("number")
            for target in MARKDOWN_LINK.findall(index_text)
            for match in [ADR_FILE.match(Path(target).name)]
            if match
        }
        self.assertEqual(
            linked,
            decision_numbers_on_disk(),
            "spec/decisions/README.md must link every decision record and nothing else",
        )

    def test_no_architecture_document_enumerates_the_decision_records(self) -> None:
        """A document may cite decisions; it may not become a second index.

        Citing one or two records to explain a design choice is normal. Linking
        most of the catalogue is an index, and a second index drifts: the
        decisions chapter fell five records behind, and once its list was
        removed the same list reappeared in the solution-strategy chapter.
        """
        offenders = []
        for path in sorted(ARCHITECTURE_DIR.glob("*.md")):
            linked = set(
                re.findall(
                    r"decisions/([0-9]{4})-[a-z0-9-]+\.md",
                    path.read_text(encoding="utf-8"),
                )
            )
            if len(linked) > MAX_DECISION_LINKS_PER_DOCUMENT:
                offenders.append(
                    f"{path.name}: links {len(linked)} decision records; "
                    "cite by ID and leave the list to spec/decisions/README.md"
                )
        self.assertEqual(offenders, [])

    def test_checklist_cites_only_records_that_exist(self) -> None:
        """The change checklist attributes each item to a registry record.

        A citation that no longer resolves is worse than no citation: it tells
        the reader a rule is written down somewhere when it is not.
        """
        checklist = (
            REPO_ROOT / "spec" / "architecture" / "change-checklist.md"
        ).read_text(encoding="utf-8")
        cited = set(re.findall(r"\b((?:INV|REQ)-[A-Z]+(?:-[A-Z]+)+)\b", checklist))
        declared = {record.id for record in all_records()}

        self.assertTrue(cited, "the checklist must attribute its items to records")
        self.assertEqual(
            sorted(cited - declared),
            [],
            "the checklist cites a registry record that does not exist",
        )

    def test_spec_index_mentions_every_architecture_document(self) -> None:
        index_text = SPEC_INDEX.read_text(encoding="utf-8")
        missing = [
            path.name
            for path in sorted((REPO_ROOT / "spec" / "architecture").glob("*.md"))
            if path.name not in index_text
        ]
        self.assertEqual(missing, [], "spec/README.md must list every architecture document")

    def test_the_retired_arc42_tree_is_not_recreated(self) -> None:
        """The twelve-slot template is gone and stays gone.

        Its numbered chapters invited filler: five of them restated the README
        and the decisions index, and two of those grew second decision lists.
        A document here is named for what it answers, not for a slot number.
        """
        self.assertFalse(
            (ARCHITECTURE_DIR / "arc42").exists(),
            "architecture documents live directly under spec/architecture/",
        )
        numbered = [
            path.name
            for path in sorted(ARCHITECTURE_DIR.glob("*.md"))
            if re.match(r"^\d{2}-", path.name)
        ]
        self.assertEqual(numbered, [], "name documents by subject, not by chapter number")


class ActiveLayerTests(unittest.TestCase):
    def active_documents(self) -> list[Path]:
        """Documents that describe the system as it is now.

        Decision records 0001-0006 and archived planning material keep their
        original wording: they are historical, and rewriting them would destroy
        the record of when a choice was made.
        """
        paths = [
            REPO_ROOT / "AGENTS.md",
            REPO_ROOT / "README.md",
            REPO_ROOT / "docs" / "design" / "README.md",
            REPO_ROOT / "docs" / "plans" / "README.md",
        ]
        for path in sorted((REPO_ROOT / "spec").rglob("*.md")):
            relative = path.relative_to(REPO_ROOT).as_posix()
            if re.match(r"^spec/decisions/000[1-6]-", relative):
                continue
            paths.append(path)
        return paths

    def test_active_documents_describe_a_two_host_product(self) -> None:
        offenders = []
        for path in self.active_documents():
            text = path.read_text(encoding="utf-8")
            for phrase in SINGLE_HOST_PHRASES:
                if phrase in text:
                    offenders.append(f"{path.relative_to(REPO_ROOT).as_posix()}: {phrase!r}")
        self.assertEqual(
            offenders,
            [],
            "ADR-0012 made the plugin serve Codex and Claude Code; active docs must say so",
        )

    def test_relative_links_resolve(self) -> None:
        offenders = []
        for path in self.active_documents():
            for target in MARKDOWN_LINK.findall(path.read_text(encoding="utf-8")):
                if target.startswith(("http://", "https://", "mailto:", "#")):
                    continue
                cleaned = target.split("#", 1)[0]
                if not cleaned:
                    continue
                resolved = (path.parent / cleaned).resolve()
                if not resolved.exists():
                    offenders.append(
                        f"{path.relative_to(REPO_ROOT).as_posix()} -> {target}"
                    )
        self.assertEqual(offenders, [])

    def test_archived_layers_are_marked_as_archived(self) -> None:
        offenders = []
        for path in ARCHIVE_INDEXES:
            if not path.exists():
                offenders.append(f"{path.relative_to(REPO_ROOT).as_posix()}: missing")
                continue
            if ARCHIVE_MARKER not in path.read_text(encoding="utf-8"):
                offenders.append(
                    f"{path.relative_to(REPO_ROOT).as_posix()}: missing archive marker"
                )
        self.assertEqual(offenders, [])


if __name__ == "__main__":
    unittest.main()
