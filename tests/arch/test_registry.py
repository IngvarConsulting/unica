"""Guards for the architecture v2 registry.

These check the *shape* of the registry, never its content. A rule about how
the system behaves belongs in a record; a rule about how records are written
belongs here.

Four properties keep the registry usable as it grows: a symbol and its path
derive from each other, every reference resolves, every rule that carries a
check names one that exists, and a decision stays short enough to be replaced
rather than amended.
"""

from __future__ import annotations

import ast
import importlib.util
import hashlib
import os
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

from tree_sitter import Language, Parser
import tree_sitter_rust

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "arch" / "registry.py"
SPEC = importlib.util.spec_from_file_location("arch_registry", SCRIPT)
REGISTRY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = REGISTRY
SPEC.loader.exec_module(REGISTRY)

ARCH_ROOT = REPO_ROOT / "arch"
ARCHIVE = REPO_ROOT / "docs" / "arch-v1"

DECISION_BODY_LIMIT = 40
SUPERPOWERS_MARKERS = ("For agentic workers", "**Goal:**", "**Tech Stack:**", "REQUIRED SUB-SKILL")

# A record must read without the tracker open. `#123` is the shorthand; the
# full URL is the same reference written longer. The lookbehind keeps HTML
# entities like `&#160;` and hex colours out of the match.
TRACKER_REFERENCES = (
    (re.compile(r"(?<![\w&])#\d+\b"), "issue reference"),
    (re.compile(r"github\.com/[\w.-]+/[\w.-]+/(?:issues|pull)/\d+"), "tracker link"),
)


def _python_declarations(tree: ast.AST, require_executable: bool) -> set[str]:
    if not require_executable:
        return {
            node.name
            for node in ast.walk(tree)
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        }

    declarations = {
        node.name
        for node in getattr(tree, "body", ())
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name.startswith("test")
    }
    for node in getattr(tree, "body", ()):
        if not isinstance(node, ast.ClassDef):
            continue
        base_names = {
            base.id
            if isinstance(base, ast.Name)
            else base.attr
            if isinstance(base, ast.Attribute)
            else ""
            for base in node.bases
        }
        if not (node.name.startswith("Test") or "TestCase" in base_names):
            continue
        declarations.update(
            child.name
            for child in node.body
            if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef))
            and child.name.startswith("test")
        )
    return declarations


def _rust_test_attribute(source: bytes, attribute_item) -> bool:
    attribute = next(
        (
            child
            for child in attribute_item.named_children
            if child.type == "attribute"
        ),
        None,
    )
    if attribute is None or not attribute.named_children:
        return False
    name = attribute.named_children[0]
    if name.type == "identifier":
        return source[name.start_byte : name.end_byte] == b"test"
    if name.type == "scoped_identifier":
        final = name.child_by_field_name("name")
        return (
            final is not None
            and source[final.start_byte : final.end_byte] == b"test"
        )
    return False


def _rust_has_attached_test_attribute(source: bytes, node) -> bool:
    if node.parent is None:
        return False
    siblings = node.parent.children
    index = next(
        (position for position, sibling in enumerate(siblings) if sibling == node),
        None,
    )
    if index is None:
        return False
    for sibling in reversed(siblings[:index]):
        if sibling.type in {"line_comment", "block_comment"}:
            continue
        if sibling.type != "attribute_item":
            break
        if _rust_test_attribute(source, sibling):
            return True
    return False


def named_evidence(path: Path, prop: str) -> list[str]:
    """Адреса проверок, названные пропом записи.

    Разбирается только фронт-маттер: имя, встреченное в прозе, проверкой не
    является. Адрес возвращается целиком — одноимённое объявление в другом
    файле это другая проверка, и принимать его за названное нельзя.
    """
    props, _ = REGISTRY.parse_front_matter(path.read_text(encoding="utf-8"))
    return REGISTRY.evidence_names(props.get(prop))


def evidence_reference_error(
    root: Path,
    reference: str,
    owner: str,
    *,
    require_executable: bool,
) -> str | None:
    """Resolve one `path::declaration` without accepting prose lookalikes."""
    path_text, separator, name = reference.partition("::")
    relative = Path(path_text)
    if not separator or not name:
        return f"{owner}: evidence must name an exact path::declaration"
    if relative.is_absolute() or ".." in relative.parts:
        return f"{owner}: evidence path {path_text} escapes the repository"
    resolved_root = root.resolve()
    target = (resolved_root / relative).resolve()
    if not target.is_relative_to(resolved_root):
        return f"{owner}: evidence path {path_text} escapes the repository"
    if not target.is_file():
        return f"{owner}: evidence file {path_text} is missing"

    if target.suffix == ".py":
        if require_executable and (
            not relative.parts
            or relative.parts[0] != "tests"
            or not relative.name.startswith("test")
        ):
            return (
                f"{owner}: Python evidence {path_text} is not a discoverable "
                "tests/test*.py module"
            )
        try:
            tree = ast.parse(target.read_text(encoding="utf-8"), filename=path_text)
        except (OSError, SyntaxError, UnicodeError) as error:
            return f"{owner}: cannot parse Python evidence {path_text}: {error}"
        declarations = _python_declarations(tree, require_executable)
    elif target.suffix == ".rs":
        try:
            source = target.read_bytes()
        except OSError as error:
            return f"{owner}: cannot read Rust evidence {path_text}: {error}"
        parser = Parser(Language(tree_sitter_rust.language()))
        tree = parser.parse(source)
        declarations = set()
        stack = [tree.root_node]
        while stack:
            node = stack.pop()
            if node.type == "function_item":
                identifier = node.child_by_field_name("name")
                body = node.child_by_field_name("body")
                if (
                    identifier is not None
                    and body is not None
                    and (
                        not require_executable
                        or _rust_has_attached_test_attribute(source, node)
                    )
                ):
                    declarations.add(
                        source[identifier.start_byte : identifier.end_byte].decode("utf-8")
                    )
            stack.extend(node.named_children)
    else:
        return f"{owner}: unsupported evidence source {path_text}"

    if name not in declarations:
        qualifier = " an executable test" if require_executable else ""
        return f"{owner}: {path_text} does not declare{qualifier} {name}"
    return None


_ATTRIBUTE_PATH = re.compile(r'\Apath\s*=\s*"(?P<path>[^"]+)"\Z')
_ATTRIBUTE_CFG = re.compile(r"\Acfg\((?P<predicate>.*)\)\Z", re.S)
_ATTRIBUTE_CFG_ATTR = re.compile(r"\Acfg_attr\((?P<arguments>.*)\)\Z", re.S)
_CFG_TOKEN = re.compile(
    r'\s*(?:(?P<word>[A-Za-z_][A-Za-z0-9_]*)|(?P<string>"(?:[^"\\]|\\.)*")|(?P<punct>[(),=]))'
)


def _cargo_manifest(path: Path) -> dict:
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        return {}


def _declared_features(manifest: dict) -> frozenset[str]:
    """Feature names the manifest defines, with the implicit ones of optional dependencies."""
    names = set(manifest.get("features", {}))
    tables = [
        manifest.get(key, {})
        for key in ("dependencies", "dev-dependencies", "build-dependencies")
    ]
    for target in manifest.get("target", {}).values():
        tables.extend(
            target.get(key, {})
            for key in ("dependencies", "dev-dependencies", "build-dependencies")
        )
    for table in tables:
        names.update(
            name
            for name, spec in table.items()
            if isinstance(spec, dict) and spec.get("optional")
        )
    return frozenset(names)


def _workspace_crates(root: Path) -> list[Path]:
    """Crate directories the Cargo workspace at `root` builds."""
    manifest = _cargo_manifest(root / "Cargo.toml")
    members = manifest.get("workspace", {}).get("members")
    if members is None:
        return [root] if "package" in manifest else []
    crates = [root] if "package" in manifest else []
    crates.extend(
        candidate
        for member in members
        for candidate in sorted(root.glob(member))
        if (candidate / "Cargo.toml").is_file()
    )
    return crates


def _crate_roots(crate: Path) -> set[Path]:
    """Root files of every target: explicit `Cargo.toml` paths and auto-discovered ones."""
    manifest = _cargo_manifest(crate / "Cargo.toml")
    package = manifest.get("package", {})
    roots: set[Path] = set()

    def add(relative: object) -> None:
        if isinstance(relative, str) and (crate / relative).is_file():
            roots.add((crate / relative).resolve())

    add(manifest.get("lib", {}).get("path", "src/lib.rs"))
    build = package.get("build", "build.rs")
    if build is not False:
        add(build)
    for kind, directory, auto in (
        ("bin", "src/bin", "autobins"),
        ("test", "tests", "autotests"),
        ("bench", "benches", "autobenches"),
        ("example", "examples", "autoexamples"),
    ):
        for target in manifest.get(kind, []):
            add(target.get("path"))
        if not package.get(auto, True):
            continue
        if kind == "bin":
            add("src/main.rs")
        for pattern in ("*.rs", "*/main.rs"):
            roots.update(path.resolve() for path in (crate / directory).glob(pattern))
    return roots


def _cfg_tokens(text: str) -> list[str] | None:
    tokens: list[str] = []
    text = text.strip()
    position = 0
    while position < len(text):
        match = _CFG_TOKEN.match(text, position)
        if match is None:
            return None
        tokens.append(match.group(match.lastgroup))
        position = match.end()
    return tokens


def _cfg_parse(
    tokens: list[str], index: int, features: frozenset[str]
) -> tuple[bool | None, int]:
    """Three-valued: True and False are settled, None depends on the configuration.

    A platform, `test`, a declared feature or an unknown name may hold in some
    configuration of the matrix, so it stays undecided. `any()`, `false` and a
    feature the manifest never declares hold nowhere, and `all`, `any` and
    `not` propagate that.
    """
    token = tokens[index]
    if token in {"all", "any", "not"} and tokens[index + 1] == "(":
        verdicts: list[bool | None] = []
        index += 2
        while tokens[index] != ")":
            verdict, index = _cfg_parse(tokens, index, features)
            verdicts.append(verdict)
            if tokens[index] == ",":
                index += 1
        index += 1
        if token == "not":
            if len(verdicts) != 1:
                raise ValueError("not() takes exactly one predicate")
            return (None if verdicts[0] is None else not verdicts[0]), index
        if token == "all":
            if False in verdicts:
                return False, index
            return (True if all(verdict is True for verdict in verdicts) else None), index
        if True in verdicts:
            return True, index
        return (False if all(verdict is False for verdict in verdicts) else None), index
    if not (token[0].isalpha() or token[0] == "_"):
        raise ValueError(f"unexpected token {token!r}")
    if index + 2 < len(tokens) and tokens[index + 1] == "=" and tokens[index + 2].startswith('"'):
        value = tokens[index + 2][1:-1]
        return (None if token != "feature" or value in features else False), index + 3
    if token == "true":
        return True, index + 1
    if token == "false":
        return False, index + 1
    return None, index + 1


def _cfg_verdict(predicate: str, features: frozenset[str]) -> bool | None:
    tokens = _cfg_tokens(predicate)
    if not tokens:
        return None
    try:
        verdict, index = _cfg_parse(tokens, 0, features)
    except (ValueError, IndexError):
        return None
    return verdict if index == len(tokens) else None


def _attached_attributes(source: bytes, node) -> list[str]:
    """Texts of the outer attributes attached to `node`, outermost first."""
    found: list[str] = []
    sibling = node.prev_named_sibling
    while sibling is not None:
        if sibling.type not in {"attribute_item", "line_comment", "block_comment"}:
            break
        if sibling.type == "attribute_item":
            attribute = next(
                (child for child in sibling.named_children if child.type == "attribute"),
                None,
            )
            if attribute is not None:
                found.append(source[attribute.start_byte : attribute.end_byte].decode("utf-8"))
        sibling = sibling.prev_named_sibling
    found.reverse()
    return found


def _cfg_gate(attributes: list[str], features: frozenset[str]) -> bool | None:
    """Conjunction of every `cfg(...)` among `attributes`; True when there is none."""
    verdicts = [
        _cfg_verdict(match.group("predicate"), features)
        for match in map(_ATTRIBUTE_CFG.match, attributes)
        if match is not None
    ]
    if False in verdicts:
        return False
    return True if all(verdict is True for verdict in verdicts) else None


def _module_path_candidates(
    attributes: list[str], features: frozenset[str]
) -> tuple[str | None, list[str]]:
    """The unconditional `path` and every `cfg_attr` path whose predicate may hold."""
    explicit: str | None = None
    conditional: list[str] = []
    for attribute in attributes:
        plain = _ATTRIBUTE_PATH.match(attribute)
        if plain is not None:
            explicit = plain.group("path")
            continue
        wrapped = _ATTRIBUTE_CFG_ATTR.match(attribute)
        if wrapped is None:
            continue
        tokens = _cfg_tokens(wrapped.group("arguments"))
        if not tokens:
            continue
        try:
            verdict, index = _cfg_parse(tokens, 0, features)
        except (ValueError, IndexError):
            continue
        if verdict is False:
            continue
        for position in range(index, len(tokens) - 2):
            if (
                tokens[position] == "path"
                and tokens[position + 1] == "="
                and tokens[position + 2].startswith('"')
            ):
                conditional.append(tokens[position + 2][1:-1])
    return explicit, conditional


def _included_literal(source: bytes, node) -> str | None:
    """The path of an `include!("...")` with one plain literal; composed forms are skipped."""
    macro = node.child_by_field_name("macro")
    if macro is None or source[macro.start_byte : macro.end_byte] != b"include":
        return None
    tokens = next(
        (child for child in node.named_children if child.type == "token_tree"), None
    )
    if tokens is None or len(tokens.named_children) != 1:
        return None
    literal = tokens.named_children[0]
    if literal.type != "string_literal":
        return None
    content = next(
        (child for child in literal.named_children if child.type == "string_content"),
        None,
    )
    if content is None:
        return None
    return source[content.start_byte : content.end_byte].decode("utf-8")


def _module_directory(file: Path, *, is_root: bool) -> Path:
    """Where a file's `mod name;` children live: mod-rs files own their directory."""
    if is_root or file.name == "mod.rs":
        return file.parent
    return file.parent / file.stem


def rust_sources_reached(root: Path) -> set[Path]:
    """Every `.rs` file some configuration compiles, starting from the target roots.

    An edge is a `mod name;` declaration — honouring `path`, `cfg_attr` paths
    and the directory nesting of inline `mod name { ... }` blocks — or an
    `include!("...")` with one literal. Conditional compilation removes an item
    before its file is resolved, so an edge or a file behind a `cfg` predicate
    that holds in no configuration is not followed; a predicate the check
    cannot settle keeps the edge. A file no edge reaches keeps its `#[test]`
    attributes and bodies, yet no target compiles it, so nothing in it ever
    runs.
    """
    parser = Parser(Language(tree_sitter_rust.language()))
    reached: set[Path] = set()
    seen: set[Path] = set()
    pending: list[tuple[Path, Path, frozenset[str]]] = []
    for crate in _workspace_crates(root):
        features = _declared_features(_cargo_manifest(crate / "Cargo.toml"))
        pending.extend(
            (crate_root, _module_directory(crate_root, is_root=True), features)
            for crate_root in _crate_roots(crate)
        )
    while pending:
        file, module_directory, features = pending.pop()
        if file in seen or not file.is_file():
            continue
        seen.add(file)
        source = file.read_bytes()
        tree_root = parser.parse(source).root_node
        inner = [
            source[attribute.start_byte : attribute.end_byte].decode("utf-8")
            for item in tree_root.named_children
            if item.type == "inner_attribute_item"
            for attribute in item.named_children
            if attribute.type == "attribute"
        ]
        if _cfg_gate(inner, features) is False:
            continue
        reached.add(file)
        stack = [(child, module_directory, 0) for child in tree_root.children]
        while stack:
            node, directory, depth = stack.pop()
            attributes = (
                _attached_attributes(source, node)
                if node.prev_named_sibling is not None
                else []
            )
            if attributes and _cfg_gate(attributes, features) is False:
                continue
            if node.type == "mod_item":
                name_node = node.child_by_field_name("name")
                body = node.child_by_field_name("body")
                if name_node is None:
                    continue
                name = source[name_node.start_byte : name_node.end_byte].decode("utf-8")
                if body is not None:
                    stack.extend(
                        (child, directory / name, depth + 1) for child in body.children
                    )
                    continue
                explicit, conditional = _module_path_candidates(attributes, features)
                base = file.parent if depth == 0 else directory
                candidates = (
                    [base / explicit]
                    if explicit is not None
                    else [directory / f"{name}.rs", directory / name / "mod.rs"]
                )
                candidates.extend(base / path for path in conditional)
                for candidate in candidates:
                    if candidate.is_file():
                        target = candidate.resolve()
                        pending.append(
                            (target, _module_directory(target, is_root=False), features)
                        )
                continue
            if node.type == "macro_invocation":
                included = _included_literal(source, node)
                if included is not None:
                    pending.append(((file.parent / included).resolve(), directory, features))
            stack.extend((child, directory, depth) for child in node.children)
    return reached


def repository_files(repo_root: Path, *pathspecs: str) -> list[Path]:
    """Files git tracks or would track under `pathspecs`; ignored files never enter.

    `Path.rglob` also returns what the desktop drops into a checkout: `.DS_Store`,
    editor swap files, a locally built binary. One such file breaks a text scan on
    a developer machine while CI, whose checkout carries none of them, stays
    green. A file git ignores is not part of the repository, so it is not part
    of a scan; an untracked file git would accept still is, exactly as with
    `rglob`. A tracked file deleted from the working tree is still listed, so
    callers keep their `is_file()` guard.
    """
    listed = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *pathspecs],
        cwd=repo_root,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    return sorted(repo_root / os.fsdecode(raw_path) for raw_path in listed if raw_path)


def archive_digests(repo_root: Path, archive: Path) -> dict[str, str]:
    """SHA-256 of every archive file, keyed by its archive-relative path.

    The manifest itself is the expectation, not a member of the archive.
    """
    manifest = archive / "MANIFEST.sha256"
    return {
        path.relative_to(archive).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in repository_files(repo_root, archive.relative_to(repo_root).as_posix())
        if path.is_file() and path != manifest
    }


def contract_record(props: dict) -> REGISTRY.Record:
    return REGISTRY.Record(
        id=props.get("id", ""),
        kind="contract",
        path=REGISTRY.ARCH_ROOT / "contracts" / "CTR.WIRE.EXAMPLE.md",
        props=props,
        body="# Example\n",
    )


def decision_record(props: dict) -> REGISTRY.Record:
    return REGISTRY.Record(
        id=props.get("id", ""),
        kind="decision",
        path=REGISTRY.ARCH_ROOT / "decisions" / "2026-08-21-example.md",
        props=props,
        body="# Example\n",
    )


def invariant_record(props: dict) -> REGISTRY.Record:
    return REGISTRY.Record(
        id=props.get("id", ""),
        kind="invariant",
        path=REGISTRY.ARCH_ROOT / "invariants" / "INV.WIRE.EXAMPLE.md",
        props=props,
        body="# Example\n",
    )


class RecordShapeTests(unittest.TestCase):
    def contract_props(self, **overrides: object) -> dict:
        props = {
            "id": "CTR.WIRE.EXAMPLE",
            "status": "active",
            "governs": "product",
            "version": "1",
            "decision": "DEC.2026-08-21.EXAMPLE",
            "producer": "scripts/arch/registry.py",
            "consumers": ["host"],
            "check": "tests/arch/test_registry.py::RecordShapeTests",
            "scope": ["wire"],
        }
        props.update(overrides)
        return props

    def active_decision(self, **overrides: object) -> REGISTRY.Record:
        props = {
            "id": "DEC.2026-08-21.EXAMPLE",
            "status": "active",
            "governs": "product",
            "realized": "scripts/arch/registry.py::validation_errors",
        }
        props.update(overrides)
        return decision_record(props)

    def test_contract_requires_scope_consumers_and_a_decision(self) -> None:
        props = {
            "id": "CTR.WIRE.EXAMPLE",
            "status": "active",
            "governs": "product",
            "version": "1",
            "producer": "src/example.rs",
            "check": (
                "tests/arch/test_registry.py::"
                "RecordShapeTests.test_contract_requires_scope_consumers_and_a_decision"
            ),
        }

        errors = REGISTRY.validation_errors([contract_record(props)])

        self.assertTrue(any("scope" in error for error in errors), errors)
        self.assertTrue(any("consumers" in error for error in errors), errors)
        self.assertTrue(any("decision" in error for error in errors), errors)

    def test_null_decision_does_not_ground_a_rule(self) -> None:
        errors = REGISTRY.validation_errors(
            [self.active_decision(), contract_record(self.contract_props(decision=None))]
        )

        self.assertTrue(any("decision does not resolve to a decision" in error for error in errors), errors)

    def test_rule_cannot_use_an_invariant_as_its_decision(self) -> None:
        invariant = invariant_record(
            {
                "id": "INV.WIRE.EXAMPLE",
                "status": "active",
                "governs": "product",
                "decision": "DEC.2026-08-21.EXAMPLE",
                "check": "tests/arch/test_registry.py::RecordShapeTests",
                "scope": ["wire"],
            }
        )
        errors = REGISTRY.validation_errors(
            [
                self.active_decision(),
                invariant,
                contract_record(self.contract_props(decision="INV.WIRE.EXAMPLE")),
            ]
        )

        self.assertTrue(any("decision does not resolve to a decision" in error for error in errors), errors)

    def test_active_rules_reference_active_decisions(self) -> None:
        superseded = self.active_decision(status="superseded")

        errors = REGISTRY.validation_errors(
            [superseded, contract_record(self.contract_props())]
        )

        self.assertTrue(any("active rule cites a non-active decision" in error for error in errors), errors)

    def test_current_rule_owner_establishes_the_rule(self) -> None:
        rule = contract_record(self.contract_props())

        missing_from_decision = REGISTRY.validation_errors([self.active_decision(), rule])
        self.assertTrue(
            any("does not establish its rule" in error for error in missing_from_decision),
            missing_from_decision,
        )

        unrelated = invariant_record(
            {
                "id": "INV.WIRE.EXAMPLE",
                "status": "active",
                "governs": "product",
                "decision": "DEC.2026-08-21.OTHER",
                "check": "tests/arch/test_registry.py::RecordShapeTests",
                "scope": ["wire"],
            }
        )
        historical_establishes = REGISTRY.validation_errors(
            [
                self.active_decision(establishes=["INV.WIRE.EXAMPLE"]),
                decision_record(
                    {
                        "id": "DEC.2026-08-21.OTHER",
                        "status": "active",
                        "governs": "product",
                        "realized": "scripts/arch/registry.py::validation_errors",
                        "establishes": ["INV.WIRE.EXAMPLE"],
                    }
                ),
                unrelated,
            ]
        )
        self.assertEqual(historical_establishes, [])

    def test_scope_and_consumers_are_non_empty_lists(self) -> None:
        errors = REGISTRY.validation_errors(
            [
                self.active_decision(),
                contract_record(self.contract_props(scope=[], consumers=[])),
            ]
        )

        self.assertTrue(any("`scope` must be a non-empty list" in error for error in errors), errors)
        self.assertTrue(any("`consumers` must be a non-empty list" in error for error in errors), errors)

    def test_contract_version_is_a_positive_integer(self) -> None:
        for version in ("0", "text"):
            with self.subTest(version=version):
                errors = REGISTRY.validation_errors(
                    [
                        self.active_decision(),
                        contract_record(self.contract_props(version=version)),
                    ]
                )

                self.assertTrue(
                    any("version must be a positive integer" in error for error in errors),
                    errors,
                )

    def test_decision_realized_is_status_dependent(self) -> None:
        planned = decision_record(
            {
                "id": "DEC.2026-08-21.EXAMPLE",
                "status": "planned",
                "governs": "product",
                "realized": None,
            }
        )
        active = decision_record(
            {
                "id": "DEC.2026-08-21.EXAMPLE",
                "status": "active",
                "governs": "product",
                "realized": None,
            }
        )
        planned_blank = decision_record(
            {
                "id": "DEC.2026-08-21.EXAMPLE",
                "status": "planned",
                "governs": "product",
                "realized": "",
            }
        )
        superseded_unbuilt = decision_record(
            {
                "id": "DEC.2026-08-21.EXAMPLE",
                "status": "superseded",
                "governs": "product",
                "realized": None,
                "superseded-by": "DEC.2026-08-22.SUCCESSOR",
            }
        )

        self.assertEqual(REGISTRY.validation_errors([planned]), [])
        self.assertEqual(REGISTRY.validation_errors([superseded_unbuilt]), [])
        self.assertTrue(
            any("missing prop `realized`" in error for error in REGISTRY.validation_errors([active]))
        )
        self.assertTrue(
            any(
                "missing prop `realized`" in error
                for error in REGISTRY.validation_errors([planned_blank])
            )
        )

    def test_decision_changes_names_existing_rules_as_a_list(self) -> None:
        scalar = self.active_decision(changes="CTR.WIRE.EXAMPLE")
        missing = self.active_decision(changes=["CTR.WIRE.MISSING"])

        scalar_errors = REGISTRY.validation_errors([scalar])
        missing_errors = REGISTRY.validation_errors([missing])

        self.assertTrue(
            any("`changes` must be a list" in error for error in scalar_errors),
            scalar_errors,
        )
        self.assertTrue(
            any("changes cites missing rule" in error for error in missing_errors),
            missing_errors,
        )

    def test_symbol_matches_its_path(self) -> None:
        """A reader who has the symbol must be able to open the file without an index."""
        offenders = [
            f"{record.relative}: id is {record.id!r}, path demands "
            f"{REGISTRY.expected_symbol(record)!r}"
            for record in REGISTRY.records()
            if record.id != REGISTRY.expected_symbol(record)
        ]
        self.assertEqual(offenders, [])

    def test_required_props_are_present(self) -> None:
        offenders = []
        for record in REGISTRY.records():
            for key in REGISTRY.REQUIRED_PROPS[record.kind]:
                if key not in record.props:
                    offenders.append(f"{record.relative}: missing prop `{key}`")
        self.assertEqual(offenders, [])

    def test_symbol_prefix_matches_its_registry(self) -> None:
        offenders = [
            f"{record.relative}: {record.id} is not a {record.kind}"
            for record in REGISTRY.records()
            if not record.id.startswith(REGISTRY.SYMBOL_PREFIX[record.kind] + ".")
        ]
        self.assertEqual(offenders, [])

    def test_no_symbol_collides_with_a_dos_device_name(self) -> None:
        """A symbol becomes a filename, and Windows refuses these outright.

        `CON` shipped as the contract prefix and broke checkout on Windows for
        the whole repository — not the file, the checkout. The rule is cheap to
        keep and impossible to notice by reading.
        """
        offenders = [
            f"{record.relative}: `{record.id.split('.')[0]}` is a DOS device name"
            for record in REGISTRY.records()
            if record.path.name.split(".")[0].upper() in REGISTRY.DOS_DEVICE_NAMES
        ]
        self.assertEqual(offenders, [])

    def test_governs_is_known(self) -> None:
        """Every record says which side it answers to.

        The two sides are paid for differently. A process rule exists to be
        rebuilt the day development gets awkward; a product rule is a promise
        someone outside already leans on, and changing it costs them. Left
        undeclared the two mix inside one area — `APP.DEPENDENCY-DIRECTION`
        governs our own layering while `APP.HIDDEN-SERVICES` governs what the
        model is allowed to see — and a reader cannot tell which discipline
        applies without deciding it again for himself.
        """
        offenders = [
            f"{record.relative}: governs {record.props.get('governs')!r}"
            for record in REGISTRY.records()
            if record.props.get("governs") not in REGISTRY.GOVERNS
        ]
        self.assertEqual(offenders, [])

    def test_status_is_known(self) -> None:
        offenders = [
            f"{record.relative}: status {record.props.get('status')!r}"
            for record in REGISTRY.records()
            if record.props.get("status") not in ("active", "planned", "superseded")
        ]
        self.assertEqual(offenders, [])

    def test_binding_and_planned_decisions_do_not_claim_the_same_state(self) -> None:
        """`active` describes the tree; an unrealized direction is `planned`.

        A product decision with no evidence cannot be presented as currently
        binding behavior. Conversely, a planned decision must not cite evidence
        that would make the separate state dishonest.
        """
        offenders = []
        for record in REGISTRY.records():
            if record.kind != "decision" and record.props.get("status") == "planned":
                offenders.append(f"{record.relative}: only decisions may be planned")
                continue
            if record.kind != "decision":
                continue
            status = record.props.get("status")
            realized = REGISTRY.evidence_names(record.props.get("realized"))
            if status == "active" and not realized:
                offenders.append(f"{record.relative}: active decision has no evidence")
            if status == "planned" and realized:
                offenders.append(f"{record.relative}: planned decision claims evidence")
        self.assertEqual(offenders, [])


class ReferenceTests(unittest.TestCase):
    def known(self) -> set[str]:
        return {record.id for record in REGISTRY.records()}

    def test_every_referenced_symbol_resolves(self) -> None:
        """A dangling reference tells the reader a record exists when it does not."""
        known, offenders = self.known(), []
        for record in REGISTRY.records():
            for key, value in record.props.items():
                if key == "id":
                    continue
                for item in value if isinstance(value, list) else [value]:
                    if isinstance(item, str) and REGISTRY.SYMBOL_ANYWHERE.fullmatch(item):
                        if item not in known:
                            offenders.append(f"{record.relative}: {key} cites {item}")
        self.assertEqual(offenders, [])

    def test_current_rule_owner_establishes_the_rule(self) -> None:
        by_id = {record.id: record for record in REGISTRY.records()}
        offenders = []
        for record in REGISTRY.records():
            if record.kind not in ("invariant", "contract"):
                continue
            decision = by_id.get(record.props.get("decision"))
            if decision is None or record.id not in (decision.props.get("establishes") or []):
                offenders.append(
                    f"{record.relative}: {record.props.get('decision')} does not establish {record.id}"
                )
        self.assertEqual(offenders, [])

    def test_record_ids_are_globally_unique_and_not_reused(self) -> None:
        found = REGISTRY.records()
        paths_by_id: dict[str, set[str]] = {}
        ids_by_path: dict[str, set[str]] = {}
        for record in found:
            paths_by_id.setdefault(record.id, set()).add(record.relative)
            ids_by_path.setdefault(record.relative, set()).add(record.id)

        history = subprocess.run(
            [
                "git",
                "rev-list",
                "--all",
                "--",
                "arch/decisions",
                "arch/invariants",
                "arch/contracts",
            ],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        commits = history.stdout.splitlines()
        for offset in range(0, len(commits), 64):
            batch = commits[offset : offset + 64]
            grep = subprocess.run(
                [
                    "git",
                    "grep",
                    "-E",
                    r"^id: (DEC|INV|CTR)\.",
                    *batch,
                    "--",
                    "arch/decisions/*.md",
                    "arch/invariants/*.md",
                    "arch/contracts/*.md",
                ],
                cwd=REPO_ROOT,
                capture_output=True,
                text=True,
            )
            self.assertIn(grep.returncode, (0, 1), grep.stderr)
            for line in grep.stdout.splitlines():
                _commit, path, declaration = line.split(":", 2)
                identifier = declaration.removeprefix("id: ").strip()
                relative = path.removeprefix("arch/")
                paths_by_id.setdefault(identifier, set()).add(relative)
                ids_by_path.setdefault(relative, set()).add(identifier)

        reused_ids = {
            identifier: sorted(paths)
            for identifier, paths in paths_by_id.items()
            if len(paths) != 1
        }
        reused_paths = {
            path: sorted(identifiers)
            for path, identifiers in ids_by_path.items()
            if len(identifiers) != 1
        }
        self.assertEqual(reused_ids, {}, "a deleted ID must not return at another path")
        self.assertEqual(reused_paths, {}, "a deleted path must not receive another ID")

    def test_supersession_is_mutual(self) -> None:
        by_id = {record.id: record for record in REGISTRY.records()}
        offenders = []
        for record in REGISTRY.records():
            for older in record.props.get("supersedes") or []:
                target = by_id.get(older)
                if target and target.props.get("superseded-by") != record.id:
                    offenders.append(f"{older} does not point back at {record.id}")
                if target and target.props.get("status") != "superseded":
                    offenders.append(f"{older} is superseded but not marked so")
        self.assertEqual(offenders, [])

    def test_every_rule_names_a_check_that_exists(self) -> None:
        """A rule whose check does not exist is a wish, not a rule.

        Both kinds that carry `check` answer for it. A contract names a consumer
        and a version: a promise to a named consumer that nothing verifies is an
        intention, and a version nothing measures drifts away from the form it
        claims to number. Decisions carry no check and are skipped.
        """
        offenders = []
        for record in REGISTRY.records():
            if record.kind == "decision":
                continue
            named = REGISTRY.evidence_names(record.props.get("check"))
            if not named:
                offenders.append(f"{record.relative}: no check named")
                continue
            for check in named:
                error = evidence_reference_error(
                    REPO_ROOT,
                    check,
                    record.relative,
                    require_executable=True,
                )
                if error:
                    offenders.append(error)
        self.assertEqual(offenders, [])

    def test_every_rust_evidence_is_compiled_from_a_crate_root(self) -> None:
        """A Rust check the compiler never reaches is prose with `#[test]` on it.

        `mod` declarations, not the file system, decide what a crate builds. A
        file dropped from its `mod` list keeps every attribute and body the
        textual resolution looks for, yet no target compiles it, so a record
        citing it names a test that has not run since the declaration went
        away. The same holds for `realized` on a decision.
        """
        reached = rust_sources_reached(REPO_ROOT)
        offenders = []
        for record in REGISTRY.records():
            prop = "realized" if record.kind == "decision" else "check"
            for evidence in REGISTRY.evidence_names(record.props.get(prop)):
                relative = Path(evidence.partition("::")[0])
                if relative.suffix != ".rs":
                    continue
                if (REPO_ROOT / relative).resolve() not in reached:
                    offenders.append(
                        f"{record.relative}: {relative.as_posix()} is not reached "
                        "from any crate root"
                    )
        self.assertEqual(offenders, [])

    def test_rust_module_graph_follows_declarations_paths_includes_and_cargo_targets(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            crate = root / "crates" / "demo"
            crate.mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/*"]\n', encoding="utf-8"
            )
            (crate / "Cargo.toml").write_text(
                '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n\n'
                '[[test]]\nname = "declared"\npath = "tests/declared/entry.rs"\n',
                encoding="utf-8",
            )
            sources = {
                "src/lib.rs": (
                    "#[cfg(test)]\nmod tests;\n"
                    '#[path = "custom/renamed.rs"]\nmod renamed;\n'
                    "mod inline { mod nested; }\n"
                ),
                "src/tests.rs": "#[test]\nfn reached() {}\n",
                "src/custom/renamed.rs": "mod deeper;\n",
                "src/custom/renamed/deeper.rs": "",
                "src/inline/nested.rs": "",
                "src/main.rs": "fn main() {}\n",
                "src/bin/tool.rs": "fn main() {}\n",
                "src/orphan.rs": "#[test]\nfn never_compiled() {}\n",
                "tests/auto.rs": 'include!("shared/body.rs");\n',
                "tests/shared/body.rs": "#[test]\nfn included() {}\n",
                "tests/shared/quoted.rs": (
                    'const FIXTURE: &str = r#"include!("shared/quoted.rs");"#;\n'
                ),
                "tests/declared/entry.rs": "mod helper;\n",
                "tests/declared/helper.rs": "",
                "benches/speed.rs": "",
            }
            for relative, text in sources.items():
                path = crate / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8")

            reached = {
                path.relative_to(crate).as_posix() for path in rust_sources_reached(root)
            }

            self.assertEqual(
                reached,
                set(sources) - {"src/orphan.rs", "tests/shared/quoted.rs"},
            )

    def test_rust_module_graph_drops_declarations_behind_a_false_cfg(self) -> None:
        """`#[cfg]` removes an item before its file is resolved: a false gate is no edge.

        A predicate the check cannot decide — a platform, `test`, a feature the
        manifest declares — keeps the edge, because some configuration of the
        matrix compiles it. A predicate false in every configuration — `any()`,
        `false`, a feature the manifest never declares, or a conjunction with
        one of them — drops the edge, on a module, an inline block, an
        `include!` and a file's own inner attribute alike. A `cfg_attr` path
        adds a candidate file unless its predicate is false.
        """
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            crate = root / "crates" / "demo"
            crate.mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/*"]\n', encoding="utf-8"
            )
            (crate / "Cargo.toml").write_text(
                '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n\n'
                "[features]\ndeclared = []\n",
                encoding="utf-8",
            )
            sources = {
                "src/lib.rs": (
                    "#[cfg(any())]\nmod disabled;\n"
                    "#[cfg(false)]\nmod literal_false;\n"
                    '#[cfg(feature = "absent")]\nmod feature_absent;\n'
                    '#[cfg(feature = "declared")]\nmod feature_declared;\n'
                    "#[cfg(test)]\n#[cfg(any())]\nmod conjunction;\n"
                    '#[cfg(all(unix, not(target_os = "macos")))]\nmod platform;\n'
                    "#[cfg(not(any()))]\nmod double_negation;\n"
                    "#[cfg(test)]\nmod tests;\n"
                    "mod inner_disabled;\n"
                    "#[cfg(any())]\nmod gated_block { mod inside; }\n"
                    '#[cfg_attr(windows, path = "alternate/windows.rs")]\nmod alternate;\n'
                    '#[cfg_attr(any(), path = "alternate/never.rs")]\nmod fallback;\n'
                ),
                "src/disabled.rs": "#[test]\nfn never() {}\n",
                "src/literal_false.rs": "",
                "src/feature_absent.rs": "",
                "src/feature_declared.rs": "",
                "src/conjunction.rs": "",
                "src/platform.rs": "",
                "src/double_negation.rs": "",
                "src/tests.rs": "#[test]\nfn compiled() {}\n",
                "src/inner_disabled.rs": "#![cfg(any())]\n#[test]\nfn never() {}\n",
                "src/gated_block/inside.rs": "",
                "src/alternate.rs": "",
                "src/alternate/windows.rs": "",
                "src/alternate/never.rs": "",
                "src/fallback.rs": "",
                "tests/auto.rs": (
                    '#[cfg(any())]\ninclude!("shared/dropped.rs");\n'
                    'include!("shared/kept.rs");\n'
                ),
                "tests/shared/dropped.rs": "#[test]\nfn never() {}\n",
                "tests/shared/kept.rs": "",
            }
            for relative, text in sources.items():
                path = crate / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8")

            reached = {
                path.relative_to(crate).as_posix() for path in rust_sources_reached(root)
            }

            self.assertEqual(
                reached,
                {
                    "src/lib.rs",
                    "src/feature_declared.rs",
                    "src/platform.rs",
                    "src/double_negation.rs",
                    "src/tests.rs",
                    "src/alternate.rs",
                    "src/alternate/windows.rs",
                    "src/fallback.rs",
                    "tests/auto.rs",
                    "tests/shared/kept.rs",
                },
            )

    def test_evidence_reference_requires_an_exact_python_or_rust_declaration(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tests = root / "tests"
            tests.mkdir()
            python = tests / "test_checks.py"
            rust = root / "checks.rs"
            python.write_text(
                "import unittest\n"
                "TEXT = 'test_only_in_a_literal'\n"
                "# def test_only_in_a_comment(): pass\n"
                "def helper_python():\n"
                "    pass\n"
                "class Checks(unittest.TestCase):\n"
                "    def test_real_python(self):\n"
                "        pass\n",
                encoding="utf-8",
            )
            rust.write_text(
                'const TEXT: &str = "test_only_in_a_literal";\n'
                "// fn test_only_in_a_comment() {}\n"
                "fn helper_rust() {}\n"
                "#[test]\n"
                "fn test_real_rust() {}\n",
                encoding="utf-8",
            )

            self.assertIsNone(
                evidence_reference_error(
                    root,
                    "tests/test_checks.py::test_real_python",
                    "fixture",
                    require_executable=True,
                )
            )
            self.assertIsNone(
                evidence_reference_error(
                    root,
                    "checks.rs::test_real_rust",
                    "fixture",
                    require_executable=True,
                )
            )
            for reference in (
                "tests/test_checks.py",
                "tests/test_checks.py::test_only_in_a_literal",
                "tests/test_checks.py::test_only_in_a_comment",
                "checks.rs::test_only_in_a_literal",
                "checks.rs::test_only_in_a_comment",
            ):
                with self.subTest(reference=reference):
                    self.assertIsNotNone(
                        evidence_reference_error(
                            root,
                            reference,
                            "fixture",
                            require_executable=True,
                        )
                    )
            for reference in (
                "tests/test_checks.py::helper_python",
                "checks.rs::helper_rust",
            ):
                with self.subTest(non_executable=reference):
                    self.assertIsNotNone(
                        evidence_reference_error(
                            root,
                            reference,
                            "fixture",
                            require_executable=True,
                        )
                    )
                    self.assertIsNone(
                        evidence_reference_error(
                            root,
                            reference,
                            "fixture",
                            require_executable=False,
                        )
                    )

    def test_executable_python_evidence_must_be_in_a_discoverable_test_module(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            arbitrary = root / "checks.py"
            arbitrary.write_text("def test_looks_executable(): pass\n", encoding="utf-8")

            error = evidence_reference_error(
                root,
                "checks.py::test_looks_executable",
                "fixture",
                require_executable=True,
            )

            self.assertIsNotNone(error)
            self.assertIn("discoverable", error or "")
            self.assertIsNone(
                evidence_reference_error(
                    root,
                    "checks.py::test_looks_executable",
                    "fixture",
                    require_executable=False,
                )
            )

    def test_executable_python_evidence_rejects_nested_test_shapes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tests = root / "tests"
            tests.mkdir()
            nested = tests / "test_nested.py"
            nested.write_text(
                "def outer():\n"
                "    def test_nested_function(): pass\n"
                "    class TestNested:\n"
                "        def test_nested_method(self): pass\n",
                encoding="utf-8",
            )

            for name in ("test_nested_function", "test_nested_method"):
                with self.subTest(name=name):
                    self.assertIsNotNone(
                        evidence_reference_error(
                            root,
                            f"tests/test_nested.py::{name}",
                            "fixture",
                            require_executable=True,
                        )
                    )

    def test_a_realized_decision_names_evidence_that_exists(self) -> None:
        """`realized` separates what was decided from what was built.

        A decision states a choice. While the tree does not match it, the
        decision is `planned` and its evidence is `null`; `active` is reserved
        for a choice with named evidence. Evidence is named the way a check is
        named, so it is verified the same way.
        """
        offenders = []
        for record in REGISTRY.records():
            if record.kind != "decision":
                continue
            for evidence in REGISTRY.evidence_names(record.props.get("realized")):
                error = evidence_reference_error(
                    REPO_ROOT,
                    evidence,
                    record.relative,
                    require_executable=False,
                )
                if error:
                    offenders.append(error)
        self.assertEqual(offenders, [])

    def test_no_rule_explains_its_own_props(self) -> None:
        """A rule speaks about its subject, not about how to read itself.

        The registry's shape is stated once, in `arch/README.md`. Copied into a
        record it lives in two places and drifts silently: the shape changes in
        the README and the copy keeps teaching the old one. It also spends the
        record's budget on an instruction manual instead of the subject.

        Only a record's *own* props are barred, and only for the two kinds that
        merely carry them. A rule about someone else's prop is doing its job:
        `INV.REGISTRY.REALIZATION-NAMED` speaks about `realized` on decisions
        and carries no such prop. A decision that introduces a prop has to name
        it, so decisions are out of scope. Own-prop scoping also keeps the
        check off the domain words that collide with prop names — `check`,
        `scope` and `design` are entries and directories here too.
        """
        offenders = []
        for record in REGISTRY.records():
            if record.kind not in ("invariant", "contract"):
                continue
            for prop in record.props:
                if prop == "id":
                    continue
                if re.search(rf"`{re.escape(prop)}\b[^`]*`", record.body):
                    offenders.append(f"{record.relative}: body explains its own `{prop}`")
        self.assertEqual(offenders, [])

    def test_every_contract_names_a_producer_that_exists(self) -> None:
        """A contract whose producer moved is lying about where the form is made."""
        offenders = []
        for record in REGISTRY.records():
            if record.kind != "contract":
                continue
            producer = record.props.get("producer") or ""
            if not producer:
                offenders.append(f"{record.relative}: no producer named")
            elif not (REPO_ROOT / producer).exists():
                offenders.append(f"{record.relative}: producer {producer} is missing")
        self.assertEqual(offenders, [])


class AtomicityTests(unittest.TestCase):
    def test_artifact_cache_decision_keys_the_path_by_artifact(self) -> None:
        """One archive shared by two tools must still have one cache root."""
        decision = (
            ARCH_ROOT / "decisions" / "2026-08-19-artifact-versioned-cache.md"
        ).read_text(encoding="utf-8")

        self.assertIn("`<артефакт>/<версия>--<sha256 архива>/<цель>`", decision)
        self.assertNotIn("`<инструмент>/<версия>--<sha256 архива>/<цель>`", decision)

    def test_a_decision_states_exactly_one_decision(self) -> None:
        offenders = [
            f"{record.relative}: {record.body.count('**Решение.**')} decision blocks"
            for record in REGISTRY.records()
            if record.kind == "decision" and record.body.count("**Решение.**") != 1
        ]
        self.assertEqual(offenders, [])

    def test_a_decision_stays_replaceable(self) -> None:
        """Past the cap a record accretes context, and context is what makes a
        decision expensive to swap. Longer reasoning belongs in `docs/design/`."""
        offenders = []
        for record in REGISTRY.records():
            if record.kind != "decision":
                continue
            lines = [line for line in record.body.splitlines() if line.strip()]
            if len(lines) > DECISION_BODY_LIMIT:
                offenders.append(f"{record.relative}: {len(lines)} lines > {DECISION_BODY_LIMIT}")
        self.assertEqual(offenders, [])


class IndexTests(unittest.TestCase):
    def test_index_matches_what_the_generator_renders(self) -> None:
        rendered = REGISTRY.render_index(REGISTRY.records())
        self.assertTrue(REGISTRY.INDEX_PATH.is_file(), "arch/index.md must exist")
        self.assertEqual(REGISTRY.INDEX_PATH.read_text(encoding="utf-8"), rendered)

    def test_generated_index_is_the_exact_registry_inventory(self) -> None:
        rendered = REGISTRY.render_index(REGISTRY.records())
        indexed_ids = re.findall(r"(?m)^\| `([^`]+)` \|", rendered)
        record_ids = [record.id for record in REGISTRY.records()]
        self.assertEqual(indexed_ids, record_ids)
        self.assertEqual(REGISTRY.INDEX_PATH.read_text(encoding="utf-8"), rendered)


class LayerBoundaryTests(unittest.TestCase):
    def test_superpowers_shapes_never_enter_arch(self) -> None:
        offenders = []
        for path in sorted(ARCH_ROOT.rglob("*.md")):
            text = path.read_text(encoding="utf-8")
            for marker in SUPERPOWERS_MARKERS:
                if marker in text:
                    offenders.append(f"{path.relative_to(REPO_ROOT).as_posix()}: {marker!r}")
        self.assertEqual(offenders, [])

    def test_no_record_points_at_the_tracker(self) -> None:
        """A record must state its ground without a second system open.

        Issue numbers are closed, renumbered and die with the repository that
        held them, so a rule whose ground is `see #574` loses its ground when
        the tracker does. The link works the other way: a symbol derives from
        its path and does not move, so a task cites one safely, and the work
        is found by searching the tracker for the symbol.
        """
        offenders = []
        for path in sorted(ARCH_ROOT.rglob("*.md")):
            text = path.read_text(encoding="utf-8")
            for pattern, what in TRACKER_REFERENCES:
                for match in pattern.finditer(text):
                    offenders.append(
                        f"{path.relative_to(ARCH_ROOT).as_posix()}: {what} {match.group(0)!r}"
                    )
        self.assertEqual(offenders, [])

    def test_archive_matches_frozen_manifest(self) -> None:
        """The archive is frozen by bytes, independently of git history shape."""
        manifest = ARCHIVE / "MANIFEST.sha256"
        self.assertTrue(manifest.is_file(), "docs/arch-v1/MANIFEST.sha256 is missing")

        expected = {}
        for line in manifest.read_text(encoding="utf-8").splitlines():
            digest, separator, relative = line.partition("  ")
            self.assertEqual(separator, "  ", f"malformed archive manifest line: {line!r}")
            expected[relative] = digest

        actual = archive_digests(REPO_ROOT, ARCHIVE)
        self.assertEqual(set(actual), set(expected), "archive file set differs from its manifest")
        self.assertEqual(actual, expected, "archive bytes differ from their frozen digests")

    def test_archive_digests_skip_what_git_ignores(self) -> None:
        """Finder's `.DS_Store` in the archive is not drift; an unstaged file still is.

        The freeze covers what git tracks or would track. An ignored file
        cannot reach a commit, so it cannot change the archive anyone
        receives, while a file dropped into the archive without `git add`
        is still reported before it is staged.
        """
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "docs" / "arch-v1"
            archive.mkdir(parents=True)
            (root / ".gitignore").write_text(".DS_Store\n", encoding="utf-8")
            (archive / "MANIFEST.sha256").write_text("", encoding="utf-8")
            (archive / "frozen.md").write_bytes(b"frozen\n")
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            (archive / ".DS_Store").write_bytes(b"\x00\x00\x00\x01Bud1\xa8\xff")
            (archive / "unstaged.md").write_bytes(b"drift\n")

            self.assertEqual(
                archive_digests(root, archive),
                {
                    "frozen.md": hashlib.sha256(b"frozen\n").hexdigest(),
                    "unstaged.md": hashlib.sha256(b"drift\n").hexdigest(),
                },
            )

    def test_v2_process_policy_changes_are_explicit_and_compatible(self) -> None:
        agents = (REPO_ROOT / "AGENTS.md").read_text(encoding="utf-8")
        self.assertIn("Проектная записка фиксирует путь к выбору и нормативной не становится", agents)
        self.assertTrue(all(record.path.is_relative_to(ARCH_ROOT) for record in REGISTRY.records()))
        self.assertFalse((REPO_ROOT / "tests/ci/test_architecture_registry.py").exists())
        self.assertFalse(any("RUSSIAN-NORMATIVE" in record.id for record in REGISTRY.records()))

    def test_archive_manifest_cannot_change_after_acceptance(self) -> None:
        """Once the freeze reaches main, a matching rewritten manifest is still drift."""
        base = subprocess.run(
            ["git", "show", "origin/main:docs/arch-v1/MANIFEST.sha256"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        if base.returncode != 0:
            self.assertFalse(
                (REPO_ROOT / "spec").exists(),
                "the initial freeze may be absent on main only after spec/ moved",
            )
            return
        self.assertEqual(
            (ARCHIVE / "MANIFEST.sha256").read_text(encoding="utf-8"),
            base.stdout,
            "the accepted archive manifest is immutable",
        )


class RetainedApplyFoundationTests(unittest.TestCase):
    def test_closed_transaction_slice_has_narrow_active_records(self) -> None:
        decision = ARCH_ROOT / "decisions/2026-08-26-retained-apply-transaction-foundation-slice.md"
        participants = ARCH_ROOT / "invariants/INV.APP.RETAINED-APPLY-CLOSED-PARTICIPANTS.md"
        rollback = ARCH_ROOT / "invariants/INV.CACHE.RETAINED-APPLY-REVISION-ROLLBACK.md"
        order = ARCH_ROOT / "invariants/INV.CACHE.RETAINED-APPLY-DETERMINISTIC-ORDER.md"
        write_free = ARCH_ROOT / "invariants/INV.SOURCE.RETAINED-APPLY-WRITE-FREE.md"

        self.assertTrue(decision.is_file())
        self.assertTrue(participants.is_file())
        self.assertTrue(rollback.is_file())
        self.assertTrue(order.is_file())
        self.assertTrue(write_free.is_file())
        self.assertIn("status: active", decision.read_text(encoding="utf-8"))
        # Запись называет сами проверки, а не обёртку над ними: обёртка лишь
        # переисполняла то, что харнесс уже прогнал.
        realized = named_evidence(decision, "realized")
        apply_rs = "crates/unica-coder/src/infrastructure/native_operations/apply.rs"
        actor_rs = "crates/unica-coder/src/infrastructure/workspace_actor.rs"
        for name in (
            f"{apply_rs}::retained_transaction_roles_require_explicit_roots_and_cache_authority",
            f"{apply_rs}::closed_transaction_rejects_physical_alias_and_second_cache_participant",
            f"{actor_rs}::prepared_apply_success_publishes_source_cache_record_and_state_as_one_revision",
        ):
            self.assertIn(name, realized)
        participant_checks = named_evidence(participants, "check")
        for name in (
            f"{apply_rs}::retained_transaction_roles_require_explicit_roots_and_cache_authority",
            f"{apply_rs}::closed_transaction_rejects_physical_alias_and_second_cache_participant",
            f"{actor_rs}::apply_admission_rejects_source_inside_cache",
        ):
            self.assertIn(name, participant_checks)
        self.assertIn(
            "retained_apply_failures_restore_source_cache_and_revision_machine_exactly",
            rollback.read_text(encoding="utf-8"),
        )
        order_checks = named_evidence(order, "check")
        for name in (
            f"{actor_rs}::prepared_apply_observer_sees_source_eager_revision_and_state_marker_order",
            f"{actor_rs}::retained_apply_observer_sees_exact_reverse_rollback_after_state_marker",
        ):
            self.assertIn(name, order_checks)
        self.assertIn(
            "apply_admission_and_dry_run_revision_observation_are_cache_tree_write_free",
            write_free.read_text(encoding="utf-8"),
        )

    def test_process_cache_rule_claims_only_application_dispatch(self) -> None:
        text = (ARCH_ROOT / "invariants/INV.CACHE.ORCHESTRATOR-OWNED.md").read_text(
            encoding="utf-8"
        )
        self.assertNotIn("отдельно", text)
        self.assertNotIn("Обработчик не публикует кеш", text)
        self.assertIn("application dispatch", text.lower())


class RetainedApplyEffectResultTests(unittest.TestCase):
    def test_retained_effect_result_slice_has_exact_active_records_and_witness(self) -> None:
        decision = (
            ARCH_ROOT
            / "decisions/2026-08-26-retained-apply-effect-publication-slice.md"
        )
        invariant = (
            ARCH_ROOT
            / "invariants/INV.CACHE.RETAINED-APPLY-EFFECT-RESULT.md"
        )

        self.assertTrue(
            decision.is_file(),
            "retained apply effect publication decision is absent",
        )
        self.assertTrue(
            invariant.is_file(),
            "retained apply effect result invariant is absent",
        )
        decision_text = decision.read_text(encoding="utf-8")
        invariant_text = invariant.read_text(encoding="utf-8")
        self.assertIn("status: active", decision_text)
        # Запись называет сами проверки. Обёртка, стоявшая здесь прежде,
        # переисполняла их и удалена; уцелевшее типовое утверждение о доступе
        # к квитанции эффектов носит теперь имя, которое его и описывает.
        actor_rs = "crates/unica-coder/src/infrastructure/workspace_actor.rs"
        for name in (
            f"{actor_rs}::prepared_apply_effects_are_retained_from_planner_to_result",
            f"{actor_rs}::prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write",
        ):
            self.assertIn(name, named_evidence(decision, "realized"))
        self.assertIn(
            "decision: DEC.2026-08-26.RETAINED-APPLY-EFFECT-PUBLICATION-SLICE",
            invariant_text,
        )
        for name in (
            f"{actor_rs}::prepared_apply_effects_are_retained_from_planner_to_result",
            f"{actor_rs}::prepared_apply_success_returns_committed_effect_receipt_after_one_commit",
        ):
            self.assertIn(name, named_evidence(invariant, "check"))
        self.assertNotIn("CTR.", decision_text)
        self.assertNotIn("wire", invariant_text.lower())

    def test_active_witness_names_real_effect_foreign_actor_and_late_gates(self) -> None:
        """Правило держат сами сценарии, а не функция, вызывающая их подряд.

        Раньше здесь разбирался Rust: свидетель обязан был звать семь
        сценариев. Звал он их вторым заходом — харнесс уже прогнал каждый
        отдельным тестом. Требование по существу прежнее и переехало туда, где
        живёт обещание: запись называет эти семь проверок поимённо.
        """
        record = named_evidence(
            ARCH_ROOT / "invariants/INV.CACHE.RETAINED-APPLY-EFFECT-RESULT.md", "check"
        )
        actor_rs = "crates/unica-coder/src/infrastructure/workspace_actor.rs"
        required = tuple(
            f"{actor_rs}::{declaration}"
            for declaration in (
                "real_effect_foreign_actor_replay_preserves_both_actor_states",
                "real_effect_mutation_lane_cancellation_preserves_exact_state",
                "real_effect_mutation_lane_deadline_preserves_exact_state",
                "real_effect_mid_scan_cancellation_preserves_exact_state",
                "real_effect_mid_scan_deadline_preserves_exact_state",
                "real_effect_after_all_postimages_cancellation_rolls_back_exact_state",
                "real_effect_after_all_postimages_deadline_rolls_back_exact_state",
            )
        )
        missing = sorted(name for name in required if name not in record)
        self.assertFalse(
            missing,
            f"active retained-effect rule is missing real-effect checks: {missing}",
        )




if __name__ == "__main__":
    unittest.main()
