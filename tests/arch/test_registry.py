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

import importlib.util
import hashlib
import re
import subprocess
import sys
import unittest
from pathlib import Path

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
            path, _, name = check.partition("::")
            target = REPO_ROOT / path
            if not target.is_file():
                offenders.append(f"{record.relative}: check file {path} is missing")
            elif name and name not in target.read_text(encoding="utf-8"):
                offenders.append(f"{record.relative}: {path} does not define {name}")
        self.assertEqual(offenders, [])

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
            path, _, name = str(evidence).partition("::")
            target = REPO_ROOT / path
            if not target.is_file():
                offenders.append(f"{record.relative}: evidence file {path} is missing")
            elif name and name not in target.read_text(encoding="utf-8"):
                offenders.append(f"{record.relative}: {path} does not define {name}")
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


if __name__ == "__main__":
    unittest.main()
