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
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

ARCHITECTURE_DIR = REPO_ROOT / "spec" / "architecture"
INVARIANTS = ARCHITECTURE_DIR / "invariants.md"
REQUIREMENTS = ARCHITECTURE_DIR / "quality-requirements.md"
DECISIONS_DIR = REPO_ROOT / "spec" / "decisions"
DECISIONS_INDEX = DECISIONS_DIR / "README.md"
SPEC_INDEX = REPO_ROOT / "spec" / "README.md"
LOGICAL_SOURCE_ACCEPTANCE = (
    REPO_ROOT
    / "spec"
    / "acceptance"
    / "logical-source-addressing-and-resource-access.md"
)
SOURCE_RESOURCE_DOMAIN = (
    REPO_ROOT / "crates" / "unica-coder" / "src" / "domain" / "source_resources.rs"
)

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
# `unittest discover` imports a module and inspects its final namespace. A regex
# for `def test_` also matches a module-level function, a helper on a plain
# class, or a line inside a comment — none of which the runner collects. The
# validator therefore proves a conservative subset of that runtime behaviour:
# an undecorated module-level `TestCase` subclass with a direct, undecorated test
# method whose class and method bindings survive. Inherited-only methods,
# wildcard imports, dynamic discovery hooks and decorators fail closed. The
# module is parsed rather than imported because importing test code a second
# time would repeat its import-time side effects.
TESTCASE_IMPORTS = {
    "unittest": {"TestCase", "IsolatedAsyncioTestCase", "FunctionTestCase"},
    "unittest.case": {"TestCase", "FunctionTestCase"},
    "unittest.async_case": {"IsolatedAsyncioTestCase"},
}


def python_module_collects_tests(path: Path) -> bool:
    """True when the module statically proves a collectable `unittest` test."""
    try:
        tree = ast.parse(path.read_text(encoding="utf-8"))
    except SyntaxError:
        return False

    unittest_modules: dict[str, str] = {}
    imported_cases: set[str] = set()
    local_cases: dict[str, set[str]] = {}
    has_dynamic_discovery_binding = False
    has_wildcard_import = False

    def replace_binding(name: str) -> None:
        nonlocal has_dynamic_discovery_binding
        unittest_modules.pop(name, None)
        imported_cases.discard(name)
        local_cases.pop(name, None)
        if name in {"load_tests", "__dir__"}:
            has_dynamic_discovery_binding = True

    def bound_names(target: ast.expr) -> set[str]:
        if isinstance(target, ast.Name):
            return {target.id}
        if isinstance(target, (ast.List, ast.Tuple)):
            return {
                name
                for element in target.elts
                for name in bound_names(element)
            }
        return set()

    class NamedExpressionBindingVisitor(ast.NodeVisitor):
        """Find walrus targets that escape a comprehension into this scope."""

        def __init__(self) -> None:
            self.names: set[str] = set()

        def visit_NamedExpr(self, node: ast.NamedExpr) -> None:
            self.names.update(bound_names(node.target))
            self.visit(node.value)

        def visit_Lambda(self, node: ast.Lambda) -> None:
            # Defaults execute in this scope; the body belongs to the lambda.
            for expression in (
                *node.args.defaults,
                *(default for default in node.args.kw_defaults if default is not None),
            ):
                self.visit(expression)

    class ScopeBindingVisitor(ast.NodeVisitor):
        """Find bindings made in one scope without entering child scopes."""

        def __init__(self) -> None:
            self.names: set[str] = set()
            self.has_wildcard_import = False

        def visit_Name(self, node: ast.Name) -> None:
            if isinstance(node.ctx, (ast.Store, ast.Del)):
                self.names.add(node.id)

        def visit_Import(self, node: ast.Import) -> None:
            self.names.update(
                alias.asname or alias.name.split(".", maxsplit=1)[0]
                for alias in node.names
            )

        def visit_ImportFrom(self, node: ast.ImportFrom) -> None:
            if any(alias.name == "*" for alias in node.names):
                self.has_wildcard_import = True
            self.names.update(
                alias.asname or alias.name
                for alias in node.names
                if alias.name != "*"
            )

        def visit_FunctionDef(self, node: ast.FunctionDef) -> None:
            self._visit_function_definition(node)

        def visit_AsyncFunctionDef(self, node: ast.AsyncFunctionDef) -> None:
            self._visit_function_definition(node)

        def _visit_function_definition(
            self, node: ast.FunctionDef | ast.AsyncFunctionDef
        ) -> None:
            self.names.add(node.name)
            for expression in (
                *node.decorator_list,
                *node.args.defaults,
                *(default for default in node.args.kw_defaults if default is not None),
            ):
                self.visit(expression)
            if node.returns is not None:
                self.visit(node.returns)
            for argument in (
                *node.args.posonlyargs,
                *node.args.args,
                *node.args.kwonlyargs,
            ):
                if argument.annotation is not None:
                    self.visit(argument.annotation)
            if (
                node.args.vararg is not None
                and node.args.vararg.annotation is not None
            ):
                self.visit(node.args.vararg.annotation)
            if (
                node.args.kwarg is not None
                and node.args.kwarg.annotation is not None
            ):
                self.visit(node.args.kwarg.annotation)

        def visit_ClassDef(self, node: ast.ClassDef) -> None:
            self.names.add(node.name)
            for expression in (
                *node.decorator_list,
                *node.bases,
                *(keyword.value for keyword in node.keywords),
            ):
                self.visit(expression)

        def visit_Lambda(self, node: ast.Lambda) -> None:
            for expression in (
                *node.args.defaults,
                *(default for default in node.args.kw_defaults if default is not None),
            ):
                self.visit(expression)

        def _visit_comprehension(
            self,
            node: ast.ListComp | ast.SetComp | ast.DictComp | ast.GeneratorExp,
        ) -> None:
            visitor = NamedExpressionBindingVisitor()
            visitor.visit(node)
            self.names.update(visitor.names)

        visit_ListComp = _visit_comprehension
        visit_SetComp = _visit_comprehension
        visit_DictComp = _visit_comprehension
        visit_GeneratorExp = _visit_comprehension

        def visit_ExceptHandler(self, node: ast.ExceptHandler) -> None:
            if node.name is not None:
                self.names.add(node.name)
            if node.type is not None:
                self.visit(node.type)
            for statement in node.body:
                self.visit(statement)

        def visit_MatchAs(self, node: ast.MatchAs) -> None:
            if node.name is not None:
                self.names.add(node.name)
            if node.pattern is not None:
                self.visit(node.pattern)

        def visit_MatchStar(self, node: ast.MatchStar) -> None:
            if node.name is not None:
                self.names.add(node.name)

        def visit_MatchMapping(self, node: ast.MatchMapping) -> None:
            if node.rest is not None:
                self.names.add(node.rest)
            for key in node.keys:
                self.visit(key)
            for pattern in node.patterns:
                self.visit(pattern)

    def scope_bindings(node: ast.AST) -> tuple[set[str], bool]:
        visitor = ScopeBindingVisitor()
        visitor.visit(node)
        return visitor.names, visitor.has_wildcard_import

    def attribute_path(node: ast.Attribute) -> tuple[str, list[str]] | None:
        attributes = [node.attr]
        value: ast.expr = node
        while isinstance(value, ast.Attribute):
            value = value.value
            if isinstance(value, ast.Attribute):
                attributes.append(value.attr)
        if not isinstance(value, ast.Name):
            return None
        return value.id, list(reversed(attributes))

    def is_testcase_base(node: ast.expr) -> bool:
        if isinstance(node, ast.Name):
            return node.id in imported_cases or node.id in local_cases
        if not isinstance(node, ast.Attribute):
            return False
        path = attribute_path(node)
        if path is None:
            return False
        root, attributes = path
        imported_module = unittest_modules.get(root)
        if imported_module is None:
            return False
        module = ".".join((imported_module, *attributes[:-1]))
        return attributes[-1] in TESTCASE_IMPORTS.get(module, set())

    def collectable_methods(node: ast.ClassDef) -> set[str]:
        methods: set[str] = set()
        for child in node.body:
            bindings, wildcard_import = scope_bindings(child)
            if wildcard_import:
                return set()
            methods.difference_update(bindings)
            if (
                isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef))
                and (child.name.startswith("test") or child.name == "runTest")
                and not child.decorator_list
            ):
                methods.add(child.name)
        return methods

    for node in tree.body:
        if isinstance(node, ast.Import):
            for alias in node.names:
                name = alias.asname or alias.name.split(".", maxsplit=1)[0]
                replace_binding(name)
                if alias.name in TESTCASE_IMPORTS:
                    imported_module = alias.name if alias.asname else "unittest"
                    unittest_modules[name] = imported_module
            continue
        if isinstance(node, ast.ImportFrom):
            for alias in node.names:
                if alias.name == "*":
                    has_wildcard_import = True
                    continue
                name = alias.asname or alias.name
                replace_binding(name)
                if (
                    node.level == 0
                    and node.module is not None
                    and alias.name in TESTCASE_IMPORTS.get(node.module, set())
                ):
                    imported_cases.add(name)
            continue
        if isinstance(node, ast.ClassDef):
            derives = any(is_testcase_base(base) for base in node.bases)
            bindings, wildcard_import = scope_bindings(node)
            has_wildcard_import = has_wildcard_import or wildcard_import
            for name in bindings:
                replace_binding(name)
            if derives and not node.decorator_list and not node.keywords:
                local_cases[node.name] = collectable_methods(node)
            continue
        bindings, wildcard_import = scope_bindings(node)
        has_wildcard_import = has_wildcard_import or wildcard_import
        for name in bindings:
            replace_binding(name)

    return (
        not has_dynamic_discovery_binding
        and not has_wildcard_import
        and any(methods for methods in local_cases.values())
    )
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

# Registry `Rule` fields and new ADR `Решение` sections carry normative text,
# and normative text is Russian (INV-DOC-RUSSIAN-NORMATIVE). Identifiers stay
# Latin, so a rule may legitimately be almost all backticked names; only the
# prose around them is checked.
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
LATIN_TOKEN = re.compile(
    r"(?<![\w-])[A-Za-z][A-Za-z0-9]*(?:[-_.:/][A-Za-z0-9]+)*(?![\w-])"
)
# Existing product and technology names that legitimately remain Latin outside
# backticks. New code/path identifiers should be backticked instead of growing
# this set; an unknown Latin word is treated as English prose.
ALLOWED_NORMATIVE_LATIN_TOKENS = {
    "Actions",
    "ADR",
    "API",
    "Bootstrap",
    "BSL",
    "Cargo",
    "CI",
    "CR",
    "CRLF",
    "Claude",
    "Code",
    "Codex",
    "DCS",
    "DSL",
    "EDT",
    "Git",
    "GitHub",
    "ID",
    "JSON",
    "JSON-RPC",
    "Job",
    "LF",
    "Linux",
    "MCP",
    "MXL",
    "Node.js",
    "Object",
    "PATH",
    "PowerShell",
    "Python",
    "QName",
    "README",
    "Release",
    "Rust",
    "SDK",
    "SHA-256",
    "Staging-PR",
    "Unica",
    "Unix",
    "Workflow",
    "Windows",
    "XML",
    "XSD",
    "application",
    "bootstrap",
    "diff",
    "git",
    "interfaces",
    "macOS",
    "override",
    "platform",
    "promotion",
    "promotion-PR",
    "pull",
    "push",
    "request",
    "runtime",
    "shell",
    "staging",
    "v1",
    "workflow",
    "writer",
}
DECISION_SECTION = re.compile(
    r"^## Решение\s*$\n(?P<body>.*?)(?=^## |\Z)",
    re.MULTILINE | re.DOTALL,
)
# ADR-0001...ADR-0006 predate the Russian-only policy and remain immutable.
# `spec/decisions/README.md` records the same closed historical exception.
LEGACY_MIXED_LANGUAGE_DECISIONS = {
    "0001",
    "0002",
    "0003",
    "0004",
    "0005",
    "0006",
}


def unexpected_normative_latin_tokens(text: str) -> set[str]:
    prose = BACKTICKED_SPAN.sub(" ", text)
    return {
        token
        for token in LATIN_TOKEN.findall(prose)
        if token not in ALLOWED_NORMATIVE_LATIN_TOKENS
        and ADR_REFERENCE.fullmatch(token) is None
        and RECORD_CITATION.fullmatch(token) is None
    }

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


def relative_to_repo(path: Path) -> str:
    """Render a path the way a reader will look it up.

    Fixture catalogues live outside the repository, so their claimants are
    reported by absolute path rather than not at all.
    """
    if path.is_relative_to(REPO_ROOT):
        return path.relative_to(REPO_ROOT).as_posix()
    return path.as_posix()


def decision_number_claims(directory: Path = DECISIONS_DIR) -> dict[str, list[Path]]:
    """Which decision files claim each ADR number, in file-name order.

    The claims stay a list on purpose. `ADR-NNNN` is the stable identity a
    reader cites, so two files sharing a number are two records claiming one
    identity — and a scan that collapses file names into a set of numbers
    cannot represent that collision, let alone report it.
    """
    claims: dict[str, list[Path]] = {}
    for path in sorted(directory.glob("*.md")):
        match = ADR_FILE.match(path.name)
        if match:
            claims.setdefault(match.group("number"), []).append(path)
    return claims


def decision_numbers_on_disk(directory: Path = DECISIONS_DIR) -> set[str]:
    return set(decision_number_claims(directory))


def indexed_decision_claims(index: Path = DECISIONS_INDEX) -> dict[str, set[str]]:
    """Which decision file names the index links, keyed by claimed number."""
    claims: dict[str, set[str]] = {}
    for target in MARKDOWN_LINK.findall(index.read_text(encoding="utf-8")):
        match = ADR_FILE.match(Path(target).name)
        if match:
            claims.setdefault(match.group("number"), set()).add(Path(target).name)
    return claims


def decision_catalogue_offenders(
    directory: Path = DECISIONS_DIR, index: Path = DECISIONS_INDEX
) -> list[str]:
    """Everything wrong with the identity bookkeeping of the decision catalogue."""
    disk_claims = decision_number_claims(directory)
    index_claims = indexed_decision_claims(index)
    offenders: list[str] = []
    # Uniqueness is proven on the raw claims, before the set conversion below:
    # a set of numbers is exactly the representation under which two 0056
    # files looked like one record and the collision passed CI silently.
    for number, paths in sorted(disk_claims.items()):
        if len(paths) > 1:
            offenders.append(
                f"ADR-{number} is claimed by {len(paths)} records: "
                + ", ".join(relative_to_repo(path) for path in paths)
            )
    for number, names in sorted(index_claims.items()):
        if len(names) > 1:
            offenders.append(
                f"{relative_to_repo(index)} links ADR-{number} as "
                f"{len(names)} different files: " + ", ".join(sorted(names))
            )
    linked = set(index_claims)
    on_disk = set(disk_claims)
    for number in sorted(on_disk - linked):
        offenders.append(f"{relative_to_repo(index)} does not link ADR-{number}")
    for number in sorted(linked - on_disk):
        offenders.append(
            f"{relative_to_repo(index)} links ADR-{number} which has no file on disk"
        )
    return offenders


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

    def test_registry_rules_are_written_in_russian(self) -> None:
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
                    unexpected = unexpected_normative_latin_tokens(value)
                    if unexpected:
                        offenders.append(
                            f"{record.where}: {field} has English prose "
                            f"{', '.join(sorted(unexpected))}"
                        )
        self.assertEqual(
            offenders,
            [],
            "normative text is Russian so one grep finds every statement of a rule",
        )

    def test_non_legacy_decisions_are_written_in_russian(self) -> None:
        offenders = []
        for path in sorted(DECISIONS_DIR.glob("*.md")):
            file_match = ADR_FILE.match(path.name)
            if (
                file_match is None
                or file_match.group("number") in LEGACY_MIXED_LANGUAGE_DECISIONS
            ):
                continue
            section = DECISION_SECTION.search(path.read_text(encoding="utf-8"))
            if section is None:
                offenders.append(f"{path.relative_to(REPO_ROOT)}: missing Решение")
                continue
            prose = BACKTICKED_SPAN.sub(" ", section.group("body")).strip()
            if not CYRILLIC.search(prose):
                offenders.append(f"{path.relative_to(REPO_ROOT)}: Решение is not Russian")
            unexpected = unexpected_normative_latin_tokens(section.group("body"))
            if unexpected:
                offenders.append(
                    f"{path.relative_to(REPO_ROOT)}: Решение has English prose "
                    f"{', '.join(sorted(unexpected))}"
                )
        self.assertEqual(offenders, [])

    def test_language_check_rejects_mixed_english_prose(self) -> None:
        text = "1. Русский. This normative decision is written in English."
        self.assertEqual(
            unexpected_normative_latin_tokens(text),
            {"This", "normative", "decision", "is", "written", "in", "English"},
        )


class PythonCheckCollectionTests(unittest.TestCase):
    def _write_module(self, directory: Path, name: str, source: str) -> Path:
        path = directory / f"test_{name}.py"
        path.write_text(source, encoding="utf-8")
        return path

    def test_rejects_testcase_classes_unittest_cannot_collect(self) -> None:
        sources = {
            "nested": """
import unittest

def factory():
    class Hidden(unittest.TestCase):
        def test_never_collected(self):
            pass
    return Hidden
""",
            "unreachable": """
import unittest

if False:
    class Hidden(unittest.TestCase):
        def test_never_collected(self):
            pass
""",
            "unrelated_base": """
class TestCase:
    pass

class NotAUnittest(TestCase):
    def test_never_collected(self):
        pass
""",
            "decorated_away": """
import unittest

def replace_with_object(_class):
    return object()

@replace_with_object
class NotAClassAtRuntime(unittest.TestCase):
    def test_never_collected(self):
        pass
""",
            "rebound": """
import unittest

class Rebound(unittest.TestCase):
    def test_never_collected(self):
        pass

Rebound = object()
""",
            "conditionally_rebound": """
import unittest

class Rebound(unittest.TestCase):
    def test_never_collected(self):
        pass

if True:
    Rebound = object()
""",
            "deleted": """
import unittest

class Deleted(unittest.TestCase):
    def test_never_collected(self):
        pass

del Deleted
""",
            "foreign_testcase": """
from unittest.fake import TestCase

class NotAUnittest(TestCase):
    def test_never_collected(self):
        pass
""",
            "load_tests_replaces_suite": """
import unittest

class Replaced(unittest.TestCase):
    def test_never_collected(self):
        pass

def load_tests(loader, tests, pattern):
    return unittest.TestSuite()
""",
            "module_dir_hides_suite": """
import unittest

class HiddenByDir(unittest.TestCase):
    def test_never_collected(self):
        pass

def __dir__():
    return []
""",
            "wildcard_import_rebinds_class": """
from unittest import TestCase

class Candidate(TestCase):
    def test_never_collected(self):
        pass

from fake_star import *
""",
            "decorated_method": """
import unittest

class NotCallable(unittest.TestCase):
    @property
    def test_never_collected(self):
        pass
""",
            "rebound_method": """
import unittest

class NotCallable(unittest.TestCase):
    def test_never_collected(self):
        pass

    test_never_collected = None
""",
            "conditionally_rebound_method": """
import unittest

class NotCallable(unittest.TestCase):
    def test_never_collected(self):
        pass

    if True:
        test_never_collected = None
""",
            "deleted_method": """
import unittest

class NotCallable(unittest.TestCase):
    def test_never_collected(self):
        pass

    del test_never_collected
""",
            "exception_target_rebinds_class": """
import unittest

class Candidate(unittest.TestCase):
    def test_never_collected(self):
        pass

try:
    raise RuntimeError
except RuntimeError as Candidate:
    pass
""",
            "match_capture_rebinds_class": """
import unittest

class Candidate(unittest.TestCase):
    def test_never_collected(self):
        pass

match object():
    case Candidate:
        pass
""",
            "match_star_rebinds_class": """
import unittest

class Candidate(unittest.TestCase):
    def test_never_collected(self):
        pass

match []:
    case [*Candidate]:
        pass
""",
            "match_mapping_rebinds_class": """
import unittest

class Candidate(unittest.TestCase):
    def test_never_collected(self):
        pass

match {}:
    case {**Candidate}:
        pass
""",
            "relative_unittest_import": """
from .unittest import TestCase

class NotAStdlibTestCase(TestCase):
    def test_never_collected(self):
        pass
""",
            "mro_masks_inherited_method": """
import unittest

class Base(unittest.TestCase):
    def test_never_collected(self):
        pass

class Mask:
    test_never_collected = None

class Candidate(Mask, Base):
    pass

del Base
""",
            "lambda_default_rebinds_class": """
import unittest

class Candidate(unittest.TestCase):
    def test_never_collected(self):
        pass

helper = lambda value=(Candidate := object): value
""",
        }
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name, source in sources.items():
                with self.subTest(name=name):
                    path = self._write_module(directory, name, source)
                    self.assertFalse(python_module_collects_tests(path))

    def test_accepts_aliased_unittest_testcase(self) -> None:
        source = """
from unittest import TestCase as CheckCase

class Collected(CheckCase):
    def test_collected(self):
        pass
"""
        with tempfile.TemporaryDirectory() as temporary:
            path = self._write_module(Path(temporary), "aliased", source)
            self.assertTrue(python_module_collects_tests(path))

    def test_accepts_run_test_method(self) -> None:
        source = """
import unittest as testing

class Collected(testing.TestCase):
    def runTest(self):
        pass
"""
        with tempfile.TemporaryDirectory() as temporary:
            path = self._write_module(Path(temporary), "run_test", source)
            self.assertTrue(python_module_collects_tests(path))

    def test_ignores_bindings_inside_nested_function_scope(self) -> None:
        sources = {
            "nested_function": """
import unittest

class Collected(unittest.TestCase):
    def test_collected(self):
        pass

if True:
    def helper():
        Collected = object()
        return Collected
""",
            "nested_class": """
import unittest

class Collected(unittest.TestCase):
    def test_collected(self):
        pass

if True:
    class Helper:
        Collected = object()
""",
            "comprehension": """
import unittest

class Collected(unittest.TestCase):
    def test_collected(self):
        pass

if True:
    shadowed = [Collected for Collected in ()]
""",
            "lambda_body": """
import unittest

class Collected(unittest.TestCase):
    def test_collected(self):
        pass

helper = lambda: (Collected := object)
""",
        }
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name, source in sources.items():
                with self.subTest(name=name):
                    path = self._write_module(directory, name, source)
                    self.assertTrue(python_module_collects_tests(path))


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


class LogicalSourceContractTests(unittest.TestCase):
    """The accepted source contract stays tied to executable evidence."""

    def setUp(self) -> None:
        self.records = {record.id: record for record in all_records()}

    def test_decision_lifecycle_activates_both_source_contracts(self) -> None:
        legacy = (
            DECISIONS_DIR / "0015-narrow-boundaries-for-code-patch-v1.md"
        ).read_text(encoding="utf-8")
        logical = (
            DECISIONS_DIR / "0021-logical-source-addressing.md"
        ).read_text(encoding="utf-8")
        resources = (
            DECISIONS_DIR / "0022-bounded-source-resource-access.md"
        ).read_text(encoding="utf-8")
        index = DECISIONS_INDEX.read_text(encoding="utf-8")
        accepted = index.split("## Принятые решения", 1)[1].split(
            "## Предложенные решения", 1
        )[0]
        proposed = index.split("## Предложенные решения", 1)[1].split("\n## ", 1)[0]

        self.assertIn("- Статус: `superseded` — заменено ADR-0021", legacy)
        self.assertIn("- Статус: `accepted`", logical)
        self.assertIn("- Статус: `accepted`", resources)
        self.assertIn("(0021-logical-source-addressing.md)", accepted)
        self.assertIn("(0022-bounded-source-resource-access.md)", accepted)
        self.assertNotIn("(0021-logical-source-addressing.md)", proposed)
        self.assertNotIn("(0022-bounded-source-resource-access.md)", proposed)

    def test_derived_source_rules_name_their_owner_and_real_check(self) -> None:
        expected = {
            "INV-SOURCE-LOGICAL-IDENTITY": (
                "ADR-0021",
                "crates/unica-coder/src/domain/source_target.rs",
            ),
            "INV-SOURCE-SNAPSHOT-BINDING": (
                "ADR-0022",
                "crates/unica-coder/src/infrastructure/platform_xml_resources.rs",
            ),
            "INV-SOURCE-ROLE-ALLOWLIST": (
                "ADR-0022",
                "crates/unica-coder/src/domain/source_resources.rs",
            ),
            "INV-MCP-SOURCE-SURFACE": (
                "ADR-0021, ADR-0022",
                "crates/unica-coder/src/application/tool_contracts.rs",
            ),
            "INV-SKILL-SOURCE-FALLBACK": (
                "ADR-0022",
                "tests/ci/test_unica_skills.py",
            ),
        }

        for identifier, (decision, check_path) in expected.items():
            with self.subTest(identifier=identifier):
                record = self.records.get(identifier)
                self.assertIsNotNone(record, f"missing {identifier}")
                self.assertEqual(record.one("Decision"), decision)
                self.assertTrue(
                    any(check_path in check for check in record.fields.get("Check", [])),
                    f"{identifier} does not name its executable check",
                )

        for identifier in (
            "INV-SOURCE-ATOMIC-PUBLISH",
            "INV-SOURCE-IDEMPOTENT-REWRITE",
        ):
            with self.subTest(identifier=identifier):
                self.assertEqual(
                    self.records[identifier].one("Decision"),
                    "ADR-0021, ADR-0022",
                )

        atomic_checks = "\n".join(
            self.records["INV-SOURCE-ATOMIC-PUBLISH"].fields.get("Check", [])
        )
        for check_path in (
            "crates/unica-coder/src/infrastructure/platform_xml_resources.rs",
            "crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs",
            "crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs",
        ):
            with self.subTest(atomic_check=check_path):
                self.assertIn(check_path, atomic_checks)

    def test_source_resource_budgets_match_the_rust_contract(self) -> None:
        record = self.records.get("REQ-PERF-SOURCE-BOUNDS")
        self.assertIsNotNone(record, "missing REQ-PERF-SOURCE-BOUNDS")
        self.assertEqual(record.one("Decision"), "ADR-0022")
        rule = record.one("Rule") or ""
        for value in ("5", "100", "50", "64", "1", "32", "128"):
            self.assertRegex(rule, rf"(?<!\d){value}(?!\d)")

        source = SOURCE_RESOURCE_DOMAIN.read_text(encoding="utf-8")
        expected_constants = {
            "SOURCE_MANIFEST_RESOURCE_MAX": "100",
            "SOURCE_RESOURCE_PAGE_LIMIT_MAX": "50",
            "SOURCE_READ_LIMIT_MAX": "64 * 1024",
            "SOURCE_REPLACEMENT_MAX_BYTES": "1024 * 1024",
            "SOURCE_SNAPSHOT_TTL_SECONDS": "5 * 60",
        }
        for name, expression in expected_constants.items():
            with self.subTest(name=name):
                self.assertRegex(
                    source,
                    rf"pub const {name}: [^=]+ = {re.escape(expression)};",
                )
        infrastructure = (
            REPO_ROOT
            / "crates"
            / "unica-coder"
            / "src"
            / "infrastructure"
            / "platform_xml_resources.rs"
        ).read_text(encoding="utf-8")
        for declaration in (
            "MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024",
            "MAX_LIVE_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024",
            "MAX_LIVE_SNAPSHOTS: usize = 64",
        ):
            self.assertIn(declaration, infrastructure)

    def test_logical_source_acceptance_is_indexed_and_traceable(self) -> None:
        self.assertTrue(LOGICAL_SOURCE_ACCEPTANCE.is_file())
        index = SPEC_INDEX.read_text(encoding="utf-8")
        self.assertIn(LOGICAL_SOURCE_ACCEPTANCE.name, index)
        text = LOGICAL_SOURCE_ACCEPTANCE.read_text(encoding="utf-8")
        for evidence in (
            "русск",
            "английск",
            "Configuration",
            "Extension",
            "ExternalProcessor",
            "ExternalReport",
            "exact",
            "prefix",
            "children",
            "legacy_target_removed",
            "providerRevision",
            "partial",
            "snapshot_expired",
            "locate",
            "replace",
            "owner",
        ):
            with self.subTest(evidence=evidence):
                self.assertIn(evidence, text)


class IndexSynchronizationTests(unittest.TestCase):
    def test_decision_index_lists_exactly_the_records_on_disk(self) -> None:
        self.assertEqual(
            decision_catalogue_offenders(),
            [],
            "spec/decisions/README.md must link every decision record and "
            "nothing else, and one number must name exactly one record",
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


class DecisionNumberCollisionTests(unittest.TestCase):
    """Two records claiming one ADR number must fail, naming both claimants.

    While a branch was being updated from main, two `0056-*.md` files with
    different decisions coexisted, both were linked from the index, and the
    guardrail stayed silent: each scan collapsed file names into a set of
    numbers before comparing, and a set cannot hold the same number twice.
    Uniqueness therefore has to be proven on the raw claims, before any set
    conversion.
    """

    def write_catalogue(
        self, directory: Path, files: list[str], links: list[str]
    ) -> Path:
        for name in files:
            (directory / name).write_text(f"# {name}\n", encoding="utf-8")
        index = directory / "README.md"
        index.write_text(
            "# Реестр решений\n\n"
            + "".join(f"- [{name}]({name})\n" for name in links),
            encoding="utf-8",
        )
        return index

    def test_two_files_claiming_one_number_are_reported_with_both_paths(self) -> None:
        first = "0056-observable-code-search.md"
        second = "0056-terminal-runtime-receipt.md"
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            index = self.write_catalogue(directory, [first, second], [first, second])
            offenders = decision_catalogue_offenders(directory, index)
            self.assertTrue(
                offenders,
                "a catalogue with two 0056 records passed the guardrail silently",
            )
            rendered = "\n".join(offenders)
            self.assertIn("ADR-0056", rendered)
            self.assertIn((directory / first).as_posix(), rendered)
            self.assertIn((directory / second).as_posix(), rendered)

    def test_index_linking_one_number_as_two_files_names_both_targets(self) -> None:
        present = "0056-observable-code-search.md"
        phantom = "0056-terminal-runtime-receipt.md"
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            index = self.write_catalogue(directory, [present], [present, phantom])
            offenders = decision_catalogue_offenders(directory, index)
            self.assertTrue(
                offenders,
                "an index claiming ADR-0056 with two file names passed silently",
            )
            rendered = "\n".join(offenders)
            self.assertIn("ADR-0056", rendered)
            self.assertIn(present, rendered)
            self.assertIn(phantom, rendered)

    def test_honest_catalogue_reports_nothing(self) -> None:
        first = "0055-smoke-checks-packaged-runtime.md"
        second = "0056-observable-code-search.md"
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            # The same file linked twice is a wording question for the index,
            # not an identity collision: one number still names one record.
            index = self.write_catalogue(
                directory, [first, second], [first, second, second]
            )
            self.assertEqual(decision_catalogue_offenders(directory, index), [])


class RuntimeArtifactFlowContractTests(unittest.TestCase):
    """The runtime bytes exercised by smoke are the bytes approved for release."""

    def setUp(self) -> None:
        self.records = all_records()

    def test_extracted_runtime_smoke_is_owned_by_the_superseding_decision(self) -> None:
        legacy = (
            DECISIONS_DIR / "0010-ci-build-cache-and-artifact-flow.md"
        ).read_text(encoding="utf-8")
        current_path = DECISIONS_DIR / "0055-smoke-proveryaet-upakovannyy-runtime.md"
        self.assertTrue(current_path.exists(), "missing ADR-0055 runtime smoke decision")
        current = current_path.read_text(encoding="utf-8")
        design = (
            REPO_ROOT / "docs/design/2026-08-12-bsl-analyzer-v0-2-67-design.md"
        ).read_text(encoding="utf-8")

        self.assertIn("- Статус: `superseded` — заменено ADR-0055", legacy)
        self.assertIn("- Статус: `accepted`", current)
        self.assertIn("извлеч", current.lower())
        self.assertIn("архив", current.lower())
        self.assertIn("- Decision: `ADR-0055`", design)
        self.assertFalse(
            any("ADR-0010" in (record.one("Decision") or "") for record in self.records),
            "active registry rules must name ADR-0055 instead of superseded ADR-0010",
        )
        self.assertTrue(
            any("ADR-0055" in (record.one("Decision") or "") for record in self.records),
            "ADR-0055 must own the derived runtime artifact-flow rules",
        )

class RlmGenerationCutoverContractTests(unittest.TestCase):
    """The builder-15 layout and transition wording stay unambiguous."""

    def setUp(self) -> None:
        self.records = {
            record.id: record for record in parse_records(INVARIANTS)
        }

    def test_generation_rule_owns_the_complete_pair_scoped_layout(self) -> None:
        record = self.records.get("INV-CACHE-GENERATION-CUTOVER")
        self.assertIsNotNone(record, "missing generation-cutover invariant")
        rule = record.one("Rule") or ""

        for token in [
            "`workspaceRoot + sourceRoot`",
            "`rlm-bsl/index-v15`",
            "`caches/rlm-bsl/index-v15/bsl_index_status.json`",
            "`locks/rlm-bsl/index-v15/bsl_index.lock`",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, rule)

    def test_glossary_delegates_generation_policy_to_its_normative_owners(self) -> None:
        glossary = (ARCHITECTURE_DIR / "glossary.md").read_text(encoding="utf-8")
        rlm_entry = glossary.split("- **RLM**", 1)[1].split("\n- **", 1)[0]

        self.assertIn("`ADR-0059`", rlm_entry)
        self.assertIn("`INV-CACHE-GENERATION-CUTOVER`", rlm_entry)
        for duplicated_policy in (
            "`rlm-bsl/index-v15`",
            "маркерами состояния и блокировки",
            "поколение построителя 14",
        ):
            with self.subTest(duplicated_policy=duplicated_policy):
                self.assertNotIn(duplicated_policy, rlm_entry)

    def test_provenance_keeps_rlm_mcp_internal_and_unica_public(self) -> None:
        provenance = (REPO_ROOT / "spec/provenance/README.md").read_text(
            encoding="utf-8"
        )
        normalized = " ".join(provenance.split())
        self.assertNotIn("опубликованный MCP API `rlm-bsl-mcp`", normalized)
        self.assertIn("внутренний MCP API поставляемого процесса", normalized)
        self.assertIn("Единственный публичный MCP-сервер — `unica`", normalized)

    def test_decision_context_marks_the_old_executable_as_pre_transition(self) -> None:
        decision = (
            DECISIONS_DIR / "0059-rlm-generacionnyy-perehod.md"
        ).read_text(encoding="utf-8")
        context = decision.split("## Контекст", 1)[1].split("## Решение", 1)[0]

        self.assertRegex(
            context,
            r"До перехода Unica поставляла[^.]*`rlm-tools-bsl`",
        )
        self.assertNotRegex(
            context,
            r"(?m)^Unica поставляет[^.]*`rlm-tools-bsl`",
        )


class RuntimeReceiptContractTests(unittest.TestCase):
    """ADR-0066 owns the fail-closed runtime and no-fallback policy."""

    def setUp(self) -> None:
        self.records = {record.id: record for record in all_records()}
        self.decision = (
            DECISIONS_DIR / "0066-terminalnyy-receipt-runtime-v-odnom-vyzove.md"
        ).read_text(encoding="utf-8")

    def test_fallback_policy_is_in_the_normative_decision_section(self) -> None:
        section = DECISION_SECTION.search(self.decision)
        self.assertIsNotNone(section, "ADR-0066 must have a Decision section")
        body = section.group("body")
        self.assertIn("`unica.build.*`", body)
        self.assertRegex(body, r"запасн\w* пут")
        self.assertRegex(
            body,
            r"(?s)`unica\.runtime\.job\.\*`[^.]*"
            r"не используется как продолжение[^.]*"
            r"запасной путь этого вызова\.",
        )

    def test_runtime_receipt_rule_covers_packaged_guidance(self) -> None:
        record = self.records.get("INV-MCP-RUNTIME-RECEIPT")
        self.assertIsNotNone(record, "missing INV-MCP-RUNTIME-RECEIPT")
        self.assertEqual(record.one("Decision"), "ADR-0074")
        self.assertIn("packaged", record.one("Scope").split(", "))
        self.assertIn(
            "tests/ci/test_unica_skills.py",
            " ".join(record.fields.get("Check", [])),
        )


class ReaderInvocationContractTests(unittest.TestCase):
    """ADR-0044 stays activated through derived executable rules."""

    def setUp(self) -> None:
        self.records = {record.id: record for record in all_records()}

    def test_decision_is_accepted_and_indexed_only_as_accepted(self) -> None:
        decision = (
            DECISIONS_DIR / "0044-readers-ne-prinimayut-preview.md"
        ).read_text(encoding="utf-8")
        index = DECISIONS_INDEX.read_text(encoding="utf-8")
        accepted = index.split("## Принятые решения", 1)[1].split(
            "## Предложенные решения", 1
        )[0]
        proposed = index.split("## Предложенные решения", 1)[1].split("\n## ", 1)[0]

        self.assertIn("- Статус: `accepted`", decision)
        self.assertIn("(0044-readers-ne-prinimayut-preview.md)", accepted)
        self.assertNotIn("(0044-readers-ne-prinimayut-preview.md)", proposed)

    def test_preview_and_typed_result_rules_name_adr_0044(self) -> None:
        preview = self.records.get("INV-MCP-PREVIEW-MUTATION-ONLY")
        self.assertIsNotNone(preview, "missing INV-MCP-PREVIEW-MUTATION-ONLY")
        self.assertEqual(preview.one("Decision"), "ADR-0044")
        preview_rule = preview.one("Rule") or ""
        for token in ["ToolExecution::Read", "ToolExecution::Mutation", "InvocationMode::Read"]:
            with self.subTest(token=token):
                self.assertIn(token, preview_rule)

        typed = self.records["INV-MCP-TYPED-RESULT"]
        self.assertIn("ADR-0044", typed.one("Decision") or "")
        self.assertIn("typed_result_missing", typed.one("Rule") or "")

    def test_change_checklist_cites_preview_rule_without_copying_it(self) -> None:
        checklist = (
            REPO_ROOT / "spec" / "architecture" / "change-checklist.md"
        ).read_text(encoding="utf-8")
        record = self.records.get("INV-MCP-PREVIEW-MUTATION-ONLY")
        self.assertIsNotNone(record, "missing INV-MCP-PREVIEW-MUTATION-ONLY")
        if record is None:
            return
        rule = record.one("Rule") or ""

        self.assertIn("INV-MCP-PREVIEW-MUTATION-ONLY", checklist)
        self.assertNotIn(rule, checklist)

    def test_building_blocks_validate_before_deriving_invocation_mode(self) -> None:
        building_blocks = (
            REPO_ROOT / "spec" / "architecture" / "building-blocks.md"
        ).read_text(encoding="utf-8")

        self.assertIn(
            "проверяет аргументы, выводит `InvocationMode`",
            building_blocks,
        )
        self.assertIn("Читатель не принимает `dryRun`", building_blocks)
        self.assertIn("`ToolExecution`", building_blocks)
        self.assertIn("`ResultContract`", building_blocks)
        self.assertNotIn(
            "разрешает `dryRun`, проверяет аргументы",
            building_blocks,
        )
        self.assertNotIn("признак мутации", building_blocks)
        self.assertNotIn("Сегодня таких две", building_blocks)

    def test_building_blocks_describe_the_typed_project_health_git_boundary(self) -> None:
        building_blocks = (
            REPO_ROOT / "spec" / "architecture" / "building-blocks.md"
        ).read_text(encoding="utf-8")

        self.assertNotIn("GitTrackingAdapter", building_blocks)
        self.assertIn("`project_health`", building_blocks)
        self.assertIn("`unica.project.status`", building_blocks)
        self.assertIn("`unica.project.map` не выполняет Git-проверок", building_blocks)

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

    def test_operational_checklist_preserves_network_policy_file_reads(self) -> None:
        """Operational locality must not disable documentation network policy.

        Documentation and standards calls do not resolve ``OperationalConfig``,
        but they still read the shared workspace files for ``network`` and
        ``providers`` before any network access.
        """
        checklist = (
            REPO_ROOT / "spec" / "architecture" / "change-checklist.md"
        ).read_text(encoding="utf-8")
        checklist_words = " ".join(checklist.split())

        self.assertIn(
            "остальные вызовы не разрешают `OperationalConfig` и не читают "
            "`[operational]`",
            checklist_words,
        )
        self.assertIn(
            "Потребители сетевой политики документации и стандартов независимо "
            "читают `network` и `providers` из тех же файлов до сетевого "
            "обращения",
            checklist_words,
        )
        self.assertNotIn(
            "остальные вызовы не читают его файловые слои", checklist_words
        )

    def test_spec_index_mentions_every_active_architecture_and_acceptance_document(
        self,
    ) -> None:
        index_text = SPEC_INDEX.read_text(encoding="utf-8")
        missing = [
            path.name
            for directory in ("architecture", "acceptance")
            for path in sorted((REPO_ROOT / "spec" / directory).glob("*.md"))
            if path.name not in index_text
        ]
        self.assertEqual(
            missing,
            [],
            "spec/README.md must list every architecture and acceptance document",
        )

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


class LogicalDiagnosticsArchitectureTests(unittest.TestCase):
    """ADR-0062..0065 stay atomic and own separate derived rules."""

    def setUp(self) -> None:
        self.records = {record.id: record for record in all_records()}

    def test_decisions_are_accepted_and_indexed_only_as_accepted(self) -> None:
        index = DECISIONS_INDEX.read_text(encoding="utf-8")
        accepted = index.split("## Принятые решения", 1)[1].split(
            "## Предложенные решения", 1
        )[0]
        proposed = index.split("## Предложенные решения", 1)[1].split(
            "\n## ", 1
        )[0]
        files = [
            "0062-tipizirovannaya-gotovnost-rlm.md",
            "0063-logicheskie-nablyudeniya-diagnostiki.md",
            "0064-neytralnaya-kompoziciya-diagnostik.md",
            "0065-yavnyy-rezhim-migracii-chitatelya.md",
        ]

        for filename in files:
            with self.subTest(filename=filename):
                decision = (DECISIONS_DIR / filename).read_text(encoding="utf-8")
                self.assertIn("- Статус: `accepted`", decision)
                self.assertIn(f"({filename})", accepted)
                self.assertNotIn(f"({filename})", proposed)

    def test_historical_records_only_name_their_replacements(self) -> None:
        typed_reader = (
            DECISIONS_DIR / "0045-typed-reader-completion-contract.md"
        ).read_text(encoding="utf-8")
        reader_bridge = (
            DECISIONS_DIR
            / "0049-most-logicheskoy-adresacii-predmetnyh-chitateley.md"
        ).read_text(encoding="utf-8")

        self.assertIn(
            "- Статус: `superseded` — заменено ADR-0062, ADR-0063 и ADR-0064",
            typed_reader,
        )
        self.assertIn(
            "- Статус: `superseded` — заменено ADR-0065", reader_bridge
        )

    def test_atomic_diagnostics_rules_have_one_exact_owner_and_real_checks(self) -> None:
        expected = {
            "INV-MCP-DIAGNOSTIC-TARGET": (
                "ADR-0063",
                ["location", "focus", "абсолют"],
                [
                    "crates/unica-coder/src/application/tool_contracts.rs",
                    "crates/unica-coder/src/application/diagnostics.rs",
                ],
            ),
            "INV-APP-DIAGNOSTIC-PROVIDERS": (
                "ADR-0064",
                ["provider", "поряд", "дедуп"],
                [
                    "crates/unica-coder/src/application/diagnostics.rs",
                    "crates/unica-coder/src/infrastructure/diagnostics.rs",
                ],
            ),
            "INV-SOURCE-READER-MIGRATION": (
                "ADR-0065",
                ["bridge", "directSwitch", "unica.code.diagnostics"],
                [
                    "tests/ci/test_architecture_registry.py",
                    "tests/ci/test_unica_skills.py",
                ],
            ),
        }

        for identifier, (decision, tokens, checks) in expected.items():
            with self.subTest(identifier=identifier):
                record = self.records.get(identifier)
                self.assertIsNotNone(record, f"missing {identifier}")
                if record is None:
                    continue
                self.assertEqual(record.one("Decision"), decision)
                rule = record.one("Rule") or ""
                for token in tokens:
                    self.assertIn(token, rule)
                rendered_checks = " ".join(record.fields.get("Check", []))
                for check in checks:
                    self.assertIn(check, rendered_checks)

    def test_rlm_readiness_replaces_adr_0045_without_absorbing_diagnostics(self) -> None:
        typed = self.records["INV-MCP-TYPED-RESULT"]
        decisions = typed.one("Decision") or ""

        self.assertIn("ADR-0062", decisions)
        self.assertNotIn("ADR-0045", decisions)
        rlm = (
            DECISIONS_DIR / "0062-tipizirovannaya-gotovnost-rlm.md"
        ).read_text(encoding="utf-8")
        for foreign_concern in ("DiagnosticProviderRegistry", "directSwitch"):
            self.assertNotIn(foreign_concern, rlm)

    def test_readiness_states_named_by_the_rule_are_the_states_the_code_has(
        self,
    ) -> None:
        """`incomplete` answers `index_pending`, so the rule must say so.

        The registry may not describe a closed matrix that the build
        contradicts: `IndexReadiness::Incomplete` is a seventh state, and
        `definition_readiness_matrix_never_reports_false_typed_success`
        proves it is retryable-pending rather than unavailable.
        """
        readiness = (REPO_ROOT / "crates" / "unica-coder" / "src"
                     / "infrastructure" / "workspace_index.rs").read_text(
            encoding="utf-8"
        )
        declared = readiness.split("pub enum IndexReadiness {", 1)[1].split("\n}", 1)[0]
        self.assertIn("Incomplete", declared, "the state under test still exists")

        rule = self.records["INV-MCP-TYPED-RESULT"].one("Rule") or ""
        decision = (
            DECISIONS_DIR / "0062-tipizirovannaya-gotovnost-rlm.md"
        ).read_text(encoding="utf-8")

        for text, where in ((rule, "INV-MCP-TYPED-RESULT"), (decision, "ADR-0062")):
            with self.subTest(where=where):
                self.assertIn("incomplete", text)

    def test_each_diagnostics_decision_keeps_one_change_axis(self) -> None:
        def decision_body(filename: str) -> str:
            text = (DECISIONS_DIR / filename).read_text(encoding="utf-8")
            return text.split("## Решение", 1)[1].split("## Неграницы", 1)[0]

        target = decision_body("0063-logicheskie-nablyudeniya-diagnostiki.md")
        providers = decision_body(
            "0064-neytralnaya-kompoziciya-diagnostik.md"
        )
        migration = decision_body(
            "0065-yavnyy-rezhim-migracii-chitatelya.md"
        )

        self.assertNotIn("legacy_target_removed", target)
        self.assertNotIn("порядку поставщика", target)
        self.assertNotIn("legacy_target_removed", providers)
        self.assertNotIn("DiagnosticProvider", migration)
        self.assertNotIn("location.kind", migration)

    def test_runtime_and_checklist_use_the_action_contract(self) -> None:
        runtime = (ARCHITECTURE_DIR / "runtime.md").read_text(encoding="utf-8")
        checklist = (ARCHITECTURE_DIR / "change-checklist.md").read_text(
            encoding="utf-8"
        )
        config_snapshot = self.records["INV-APP-CONFIG-SNAPSHOT"]

        self.assertIn("`action=analyze`", runtime)
        self.assertNotIn("diagnostics` в режиме `analyze`", runtime)
        self.assertEqual(
            config_snapshot.one("Decision"), "ADR-0040, ADR-0056, ADR-0058, ADR-0063"
        )
        for identifier in (
            "INV-MCP-DIAGNOSTIC-TARGET",
            "INV-APP-DIAGNOSTIC-PROVIDERS",
            "INV-SOURCE-READER-MIGRATION",
        ):
            self.assertIn(identifier, checklist)
        for artifact in (
            "схем",
            "обработчик",
            "скилл",
            "примеры",
            "ведомость",
            "миграц",
            "релиз",
            "тест",
        ):
            self.assertIn(artifact, checklist.lower())

class RlmStandalonePackagingContractTests(unittest.TestCase):
    def test_accepted_decision_owns_the_multifile_runtime_choice(self) -> None:
        decision = (
            REPO_ROOT
            / "spec"
            / "decisions"
            / "0061-rlm-mnogofaylovyy-runtime-iz-proveryaemogo-arhiva.md"
        ).read_text(encoding="utf-8")

        self.assertIn("- Статус: `accepted`", decision)
        self.assertIn("`Nuitka standalone multidist`", decision)
        self.assertIn("`runtimeFiles`", decision)
        self.assertIn("скачивает общий архив цели ровно один раз", decision)
        self.assertIn("Bootstrap остаётся нейтральным", decision)

    def test_registry_rule_owns_the_exact_runtime_closure(self) -> None:
        records = {record.id: record for record in all_records()}
        record = records["INV-PKG-TOOL-CLOSURE"]
        rendered = record.one("Rule") or ""

        self.assertEqual(record.one("Decision"), "ADR-0061")
        for contract in (
            "полная полезная нагрузка закреплённого архива",
            "только обычные файлы",
            "точный набор",
            "потерянная зависимость",
            "необъявленный файл",
        ):
            with self.subTest(contract=contract):
                self.assertIn(contract, rendered)


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
