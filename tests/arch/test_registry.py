"""Guards for the architecture v2 registry.

These check the *shape* of the registry, never its content. A rule about how
the system behaves belongs in a record; a rule about how records are written
belongs here.

Four properties keep the registry usable as it grows: a symbol and its path
derive from each other, every reference resolves, every invariant names a check
that exists, and a decision stays short enough to be replaced rather than
amended.
"""

from __future__ import annotations

import importlib.util
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


class RecordShapeTests(unittest.TestCase):
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

    def test_status_is_known(self) -> None:
        offenders = [
            f"{record.relative}: status {record.props.get('status')!r}"
            for record in REGISTRY.records()
            if record.props.get("status") not in ("active", "superseded")
        ]
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

    def test_every_invariant_names_a_check_that_exists(self) -> None:
        """An invariant whose check does not exist is a wish, not an invariant."""
        offenders = []
        for record in REGISTRY.records():
            if record.kind != "invariant":
                continue
            check = record.props.get("check") or ""
            path, _, name = check.partition("::")
            target = REPO_ROOT / path
            if not target.is_file():
                offenders.append(f"{record.relative}: check file {path} is missing")
            elif name and name not in target.read_text(encoding="utf-8"):
                offenders.append(f"{record.relative}: {path} does not define {name}")
        self.assertEqual(offenders, [])


class AtomicityTests(unittest.TestCase):
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

    def test_archived_records_are_not_edited_after_the_freeze(self) -> None:
        """The freeze protects records that existed, not the tree as a shape.

        Three things may still touch `docs/arch-v1`. `FATE.md` is metadata about
        the archive rather than a record inside it. A record written against the
        v1 format on `main` before the freeze reached it arrives by merge — that
        is history landing late, not history being rewritten, and it must appear
        in `FATE.md` so it does not sit there undisposed. Everything that
        existed at the freeze answers what was decided on its date, and editing
        that after the fact destroys the only reason to keep the tree.
        """
        freeze = subprocess.run(
            ["git", "log", "--format=%H", "--grep", "move spec/ to docs/arch-v1",
             "--", "docs/arch-v1"],
            cwd=REPO_ROOT, capture_output=True, text=True, check=True,
        ).stdout.split()
        self.assertTrue(freeze, "the freeze commit must be findable")
        frozen = set(subprocess.run(
            ["git", "ls-tree", "-r", "--name-only", freeze[0], "--", "docs/arch-v1"],
            cwd=REPO_ROOT, capture_output=True, text=True, check=True,
        ).stdout.split())
        changed = set(subprocess.run(
            ["git", "log", "--format=", "--name-only", f"{freeze[0]}..HEAD",
             "--", "docs/arch-v1"],
            cwd=REPO_ROOT, capture_output=True, text=True, check=True,
        ).stdout.split())

        # The surface ledger is generated from the built binary and still lives
        # in the archive until CTR.WIRE.TOOL-SURFACE grows a home here. It is
        # output, not a record: freezing it would freeze the generator.
        writable = {"docs/arch-v1/FATE.md", "docs/arch-v1/architecture/tool-surface.md"}
        edited = sorted(p for p in changed & frozen if p not in writable)
        self.assertEqual(edited, [], "records that existed at the freeze were edited")

        fate = (REPO_ROOT / "docs" / "arch-v1" / "FATE.md").read_text(encoding="utf-8")
        undisposed = sorted(
            p for p in changed - frozen
            if p != "docs/arch-v1/FATE.md" and Path(p).stem.split("-")[0] not in fate
        )
        self.assertEqual(undisposed, [], "a record arrived in the archive without a disposition")


if __name__ == "__main__":
    unittest.main()
