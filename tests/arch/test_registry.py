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
import re
import subprocess
import sys
import tempfile
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

    def test_rule_ownership_is_reciprocal(self) -> None:
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
        stale_establishes = REGISTRY.validation_errors(
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
        self.assertTrue(
            any("establishes a rule owned by" in error for error in stale_establishes),
            stale_establishes,
        )

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

        self.assertEqual(REGISTRY.validation_errors([planned]), [])
        self.assertTrue(
            any("missing prop `realized`" in error for error in REGISTRY.validation_errors([active]))
        )
        self.assertTrue(
            any(
                "missing prop `realized`" in error
                for error in REGISTRY.validation_errors([planned_blank])
            )
        )

    def test_decision_changes_names_existing_contracts_as_a_list(self) -> None:
        scalar = self.active_decision(changes="CTR.WIRE.EXAMPLE")
        missing = self.active_decision(changes=["CTR.WIRE.MISSING"])

        scalar_errors = REGISTRY.validation_errors([scalar])
        missing_errors = REGISTRY.validation_errors([missing])

        self.assertTrue(
            any("`changes` must be a list" in error for error in scalar_errors),
            scalar_errors,
        )
        self.assertTrue(
            any("changes cites missing contract" in error for error in missing_errors),
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
            realized = record.props.get("realized")
            if status == "active" and realized in (None, ""):
                offenders.append(f"{record.relative}: active decision has no evidence")
            if status == "planned" and realized not in (None, ""):
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

    def test_rule_decision_and_establishes_are_reciprocal(self) -> None:
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
        for decision in (record for record in REGISTRY.records() if record.kind == "decision"):
            for rule_id in decision.props.get("establishes") or []:
                rule = by_id.get(rule_id)
                if rule is not None and rule.props.get("decision") != decision.id:
                    offenders.append(
                        f"{decision.relative}: establishes {rule_id}, owned by "
                        f"{rule.props.get('decision')}"
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
            check = record.props.get("check") or ""
            if not check:
                offenders.append(f"{record.relative}: no check named")
                continue
            error = evidence_reference_error(
                REPO_ROOT,
                check,
                record.relative,
                require_executable=True,
            )
            if error:
                offenders.append(error)
        self.assertEqual(offenders, [])

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
            evidence = record.props.get("realized")
            if evidence in (None, ""):
                continue
            error = evidence_reference_error(
                REPO_ROOT,
                str(evidence),
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

        actual = {
            path.relative_to(ARCHIVE).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in ARCHIVE.rglob("*")
            if path.is_file() and path != manifest
        }
        self.assertEqual(set(actual), set(expected), "archive file set differs from its manifest")
        self.assertEqual(actual, expected, "archive bytes differ from their frozen digests")

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
        self.assertIn(
            "retained_apply_transaction_foundation_contract_is_complete",
            decision.read_text(encoding="utf-8"),
        )
        self.assertIn(
            "retained_apply_closed_participant_contract_is_complete",
            participants.read_text(encoding="utf-8"),
        )
        self.assertIn(
            "retained_apply_failures_restore_source_cache_and_revision_machine_exactly",
            rollback.read_text(encoding="utf-8"),
        )
        self.assertIn(
            "retained_apply_deterministic_success_and_rollback_order_is_complete",
            order.read_text(encoding="utf-8"),
        )
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
        self.assertIn(
            "retained_apply_effect_result_contract_is_complete",
            decision_text,
        )
        self.assertIn(
            "decision: DEC.2026-08-26.RETAINED-APPLY-EFFECT-PUBLICATION-SLICE",
            invariant_text,
        )
        self.assertIn(
            "retained_apply_effect_result_contract_is_complete",
            invariant_text,
        )
        self.assertNotIn("CTR.", decision_text)
        self.assertNotIn("wire", invariant_text.lower())

    def test_active_witness_invokes_real_effect_foreign_actor_and_late_gates(self) -> None:
        source = (
            REPO_ROOT / "crates/unica-coder/src/infrastructure/workspace_actor.rs"
        ).read_bytes()
        parser = Parser(Language(tree_sitter_rust.language()))
        tree = parser.parse(source)
        stack = [tree.root_node]
        witness = None
        while stack:
            node = stack.pop()
            if node.type == "function_item":
                name = node.child_by_field_name("name")
                if (
                    name is not None
                    and source[name.start_byte : name.end_byte]
                    == b"retained_apply_effect_result_contract_is_complete"
                ):
                    witness = node.child_by_field_name("body")
                    break
            stack.extend(node.named_children)
        self.assertIsNotNone(witness, "active retained-effect witness is absent")

        calls = set()
        stack = [witness]
        while stack:
            node = stack.pop()
            if node.type == "call_expression":
                function = node.child_by_field_name("function")
                if function is not None and function.type == "identifier":
                    calls.add(source[function.start_byte : function.end_byte].decode())
            stack.extend(node.named_children)

        required = {
            "real_effect_foreign_actor_replay_preserves_both_actor_states",
            "real_effect_mutation_lane_cancellation_preserves_exact_state",
            "real_effect_mutation_lane_deadline_preserves_exact_state",
            "real_effect_mid_scan_cancellation_preserves_exact_state",
            "real_effect_mid_scan_deadline_preserves_exact_state",
            "real_effect_after_all_postimages_cancellation_rolls_back_exact_state",
            "real_effect_after_all_postimages_deadline_rolls_back_exact_state",
        }
        self.assertFalse(
            required - calls,
            f"active retained-effect witness is missing real-effect calls: {sorted(required - calls)}",
        )


if __name__ == "__main__":
    unittest.main()
