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

import ast
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

# What each automated check class must point at for the entry to mean anything.
# The registry claims a named check holds the rule; a path that merely exists
# proves nothing, so the target has to be an artefact a CI runner collects.
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
# `.github/workflows/unica-plugin-release.yml` runs `unittest discover` over
# these two trees, with the default `test*.py` pattern.
PYTHON_TEST_ROOTS = ("tests/ci/", "tests/dev/")
# `unittest discover` collects test methods of `TestCase` subclasses and nothing
# else. A regex for `def test_` also matches a module-level function, a helper on
# a plain class, or a line inside a comment — none of which the runner collects,
# so the registry would still be counting a check that never runs. The module is
# parsed rather than imported: importing a test module to inspect it runs its
# import-time code, which is a poor trade for a validator.
TESTCASE_BASE_NAMES = {"TestCase", "IsolatedAsyncioTestCase", "FunctionTestCase"}


def python_module_collects_tests(path: Path) -> bool:
    """True when `unittest` would collect at least one test from this module."""
    try:
        tree = ast.parse(path.read_text(encoding="utf-8"))
    except SyntaxError:
        return False

    def is_testcase_base(node: ast.expr) -> bool:
        name = node.attr if isinstance(node, ast.Attribute) else getattr(node, "id", None)
        return name in TESTCASE_BASE_NAMES

    local_cases = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.ClassDef):
            continue
        derives = any(
            is_testcase_base(base)
            or (isinstance(base, ast.Name) and base.id in local_cases)
            for base in node.bases
        )
        if not derives:
            continue
        local_cases.add(node.name)
        if any(
            isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef))
            and child.name.startswith("test")
            for child in node.body
        ):
            return True
    return False
# `cargo test --workspace` collects `#[test]` and `#[tokio::test]`.
RUST_TEST_ATTRIBUTE = re.compile(r"#\[(?:tokio::)?test\]")
# A Rust test target may be a two-line shim that pulls in the real file, which
# is how the platform-specific suites are laid out.
RUST_INCLUDE = re.compile(
    r'#\[path\s*=\s*"(?P<path>[^"]+)"\]|include!\(\s*"(?P<include>[^"]+)"\s*\)'
)

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
# A citation of a record, as a reader would recognise one. Digits are matched on
# purpose even though `RECORD_ID` forbids them: the identifiers this corpus has
# actually left behind are the numbered ones the registry rework replaced
# (`INV-CACHE-07`), and a resolver that cannot see them cannot report them.
RECORD_CITATION = re.compile(r"\b((?:INV|REQ)-[A-Z][A-Z0-9]*(?:-[A-Z0-9]+)+)\b")
# Names shaped like a record but written to be read as an example, never as a
# citation. `AGENTS.md` teaches the naming rule by contrasting a good code with
# a bad one, and the bad one has to look real to make the point. The resolver
# below requires every entry here to be used and to stay unclaimed, so the
# exemption cannot outlive the sentence it was written for.
ILLUSTRATIVE_IDENTIFIERS = {"INV-MCP-TOOL-CONTRACTS-RS"}
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

# Uniqueness is checked everywhere a heading could collide with a live
# identifier, the archive included. Resolution is not: see `registry_ids()`.
DECLARATION_SCAN_ROOTS = (REPO_ROOT / "spec", REPO_ROOT / "docs")

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


def registry_ids() -> set[str]:
    """Identifiers the registry actually declares — the only ones a citation may name."""
    return {record.id for record in all_records()}


def citation_corpus() -> list[Path]:
    """Documents whose record citations a reader is entitled to follow.

    The whole active specification layer, plus the two documents outside it
    that route a reader into the registry. `docs/design/**` is deliberately
    absent: it is archived planning material that keeps its original wording,
    including identifiers the registry has since replaced.
    """
    entry_points = (
        REPO_ROOT / "AGENTS.md",
        REPO_ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md",
    )
    paths = list((REPO_ROOT / "spec").rglob("*.md"))
    paths.extend(path for path in entry_points if path.is_file())
    return sorted(paths)


def rust_defines_tests(path: Path, seen: set[Path] | None = None) -> bool:
    """True when the file, or a file it inlines, declares a Rust test."""
    seen = set() if seen is None else seen
    if path in seen or not path.is_file():
        return False
    seen.add(path)
    text = path.read_text(encoding="utf-8")
    if RUST_TEST_ATTRIBUTE.search(text):
        return True
    for match in RUST_INCLUDE.finditer(text):
        relative = match.group("path") or match.group("include")
        if rust_defines_tests((path.parent / relative).resolve(), seen):
            return True
    return False


def rust_target_is_built(relative: str, path: Path) -> bool:
    """True when `cargo test --workspace` compiles the file.

    Unit tests under `src/` and the integration targets directly under
    `tests/` are compiled by name. A file deeper under `tests/` is only
    compiled when a target inlines it, which is how the platform suites and
    the two-line contract shims are arranged.
    """
    parts = relative.split("/")
    if len(parts) < 4 or parts[0] != "crates":
        return False
    if parts[2] == "src":
        return True
    if parts[2] != "tests":
        return False
    if len(parts) == 4:
        return True
    for target in sorted((REPO_ROOT / parts[0] / parts[1] / "tests").glob("*.rs")):
        text = target.read_text(encoding="utf-8")
        for match in RUST_INCLUDE.finditer(text):
            inlined = match.group("path") or match.group("include")
            if (target.parent / inlined).resolve() == path:
                return True
    return False


def ci_invocations() -> str:
    """Everything CI runs, as one blob to search for a script name.

    A guard script earns its class by being executed: from a workflow step or
    from the test suite that CI runs. A script no runner mentions is a file,
    not a check.
    """
    blobs = []
    for root in (WORKFLOWS_DIR, REPO_ROOT / "tests" / "ci", REPO_ROOT / "tests" / "dev"):
        for path in sorted(root.rglob("*")):
            if path.suffix in {".yml", ".yaml", ".py"} and path.is_file():
                blobs.append(path.read_text(encoding="utf-8"))
    return "\n".join(blobs)


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
        roots = list(DECLARATION_SCAN_ROOTS)
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

    def retired_ids(self) -> set[str]:
        """Identifiers the registry has withdrawn from circulation."""
        text = INVARIANTS.read_text(encoding="utf-8")
        section = re.search(
            r"## Выведенные из обращения идентификаторы\n(?P<body>.*?)(?=\n## |\Z)",
            text,
            re.DOTALL,
        )
        if section is None:
            return set()
        return set(RECORD_CITATION.findall(section.group("body")))

    def test_every_citation_in_the_active_corpus_resolves(self) -> None:
        """Every `INV-*`/`REQ-*` in the active layer names a record that exists.

        Scoping this to the change checklist was the hole that let nine dead
        citations survive in two decision records: a reader following
        `INV-CACHE-07` from ADR-0014 lands on nothing, and the registry rework
        that renamed it had no way to notice. A citation either resolves to a
        declared record or to the retired-identifier ledger, which is the one
        place a withdrawn identifier keeps its meaning.

        The two entry-point documents outside `spec/` carry citations too, and
        a dead identifier costs the same there: `AGENTS.md` is what an agent
        reads first, so a rename that misses it sends every reader to a rule
        that is not written down.
        """
        # Resolution accepts only what the registry declares. `declared_ids()`
        # sweeps the archive too, because a heading there would collide with a
        # live identifier and that collision has to surface — but an archived
        # design note is not a place a citation may land, so reusing that wider
        # set here would let a rule "exist" outside the normative layer.
        declared = registry_ids()
        known = declared | self.retired_ids()
        used_exemptions = set()
        offenders = []
        for path in citation_corpus():
            for number, line in enumerate(
                path.read_text(encoding="utf-8").splitlines(), start=1
            ):
                for identifier in RECORD_CITATION.findall(line):
                    if identifier in ILLUSTRATIVE_IDENTIFIERS:
                        used_exemptions.add(identifier)
                        continue
                    if identifier not in known:
                        where = f"{path.relative_to(REPO_ROOT).as_posix()}:{number}"
                        offenders.append(f"{where} cites {identifier}")
        self.assertEqual(
            offenders,
            [],
            "a citation that resolves to nothing tells the reader a rule is "
            "written down when it is not",
        )
        self.assertEqual(
            sorted(ILLUSTRATIVE_IDENTIFIERS - used_exemptions),
            [],
            "an exemption nobody uses is a licence waiting to be claimed by an "
            "unrelated identifier; drop it from ILLUSTRATIVE_IDENTIFIERS",
        )
        self.assertEqual(
            sorted(ILLUSTRATIVE_IDENTIFIERS & known),
            [],
            "an exempted name became a real record; it is now a citation like "
            "any other and must not keep a licence to resolve to nothing",
        )

    def test_retired_identifiers_are_not_reissued(self) -> None:
        """A retired identifier stays retired.

        The registry records withdrawn identifiers under a dedicated heading so
        that a future rule cannot quietly inherit the meaning of a deleted one.
        """
        text = INVARIANTS.read_text(encoding="utf-8")
        self.assertIsNotNone(
            re.search(
                r"## Выведенные из обращения идентификаторы\n(?P<body>.*?)(?=\n## |\Z)",
                text,
                re.DOTALL,
            ),
            "invariants.md must carry a retired-identifier ledger",
        )
        active = set(self.declared_ids())
        reissued = sorted(self.retired_ids() & active)
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

    def test_automated_checks_name_something_ci_actually_executes(self) -> None:
        """An automated class must point at an artefact a CI runner collects.

        Existence of the target file proves nothing: `ci-test` — `README.md`
        would pass such a test, and the registry would still claim the rule is
        held automatically. What makes the claim true is that the named file is
        a test the suite collects, or a script a runner executes. This test
        proves that for every automated check line, so the count of automated
        checks in the registry is a count of checks that run.
        """
        invocations = ci_invocations()
        offenders = []
        for record, cls, target in self.parsed_checks():
            if cls == "manual":
                continue
            backticked = BACKTICKED.match(target.strip())
            if not backticked:
                continue  # reported by the well-formedness test
            relative = backticked.group("target")
            path = REPO_ROOT / relative
            if not path.is_file():
                continue  # reported by the existence test

            if cls in {"ci-test", "doc-assert"}:
                if relative.endswith(".py"):
                    if not relative.startswith(PYTHON_TEST_ROOTS) or not path.name.startswith(
                        "test"
                    ):
                        offenders.append(
                            f"{record.where}: {relative} is outside what "
                            "`unittest discover` collects"
                        )
                    elif not python_module_collects_tests(path):
                        offenders.append(
                            f"{record.where}: {relative} defines no test that "
                            "`unittest discover` collects"
                        )
                elif relative.endswith(".rs"):
                    if not rust_target_is_built(relative, path):
                        offenders.append(
                            f"{record.where}: {relative} is not compiled by `cargo test`"
                        )
                    elif not rust_defines_tests(path):
                        offenders.append(f"{record.where}: {relative} declares no Rust test")
                else:
                    offenders.append(
                        f"{record.where}: {cls} target must be a test file, got {relative}"
                    )
            elif cls == "guard-script":
                if not relative.startswith("scripts/"):
                    offenders.append(
                        f"{record.where}: guard-script must live under scripts/, got {relative}"
                    )
                elif path.name not in invocations:
                    offenders.append(
                        f"{record.where}: no workflow or test runs {relative}"
                    )
            elif cls == "release-gate":
                workflow = relative.startswith(".github/workflows/")
                if not workflow and not relative.startswith("scripts/"):
                    offenders.append(
                        f"{record.where}: release-gate must be a workflow or a script, "
                        f"got {relative}"
                    )
                elif not workflow and path.name not in invocations:
                    offenders.append(f"{record.where}: no workflow runs {relative}")
        self.assertEqual(
            offenders,
            [],
            "an automated check class is a promise that CI runs the named target",
        )

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
        cited = set(RECORD_CITATION.findall(checklist))
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
