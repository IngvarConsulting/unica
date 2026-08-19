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
ARRIVALS_HEADING = "## Что приехало после заморозки"
SUPERPOWERS_MARKERS = ("For agentic workers", "**Goal:**", "**Tech Stack:**", "REQUIRED SUB-SKILL")

# A record must read without the tracker open. `#123` is the shorthand; the
# full URL is the same reference written longer. The lookbehind keeps HTML
# entities like `&#160;` and hex colours out of the match.
TRACKER_REFERENCES = (
    (re.compile(r"(?<![\w&])#\d+\b"), "issue reference"),
    (re.compile(r"github\.com/[\w.-]+/[\w.-]+/(?:issues|pull)/\d+"), "tracker link"),
)


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

        A decision states a choice; `status: active` says the choice holds, not
        that the tree matches it. Without a second axis the two are one word,
        and a reader takes an unbuilt decision for a description of the system.
        Evidence is named the way a check is named, so it is verified the same
        way; `null` is the honest value while the work is ahead.
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

    def test_archive_drift_is_recorded(self) -> None:
        """Every difference from the freeze is named in `FATE.md`.

        The rule used to be "the archive is not edited", checked by walking
        commits. It saw nothing: `git log --name-only` skips merges, and `main`
        still writes into `spec/`, a path the `docs/arch-v1` filter never
        matches. Seven files had drifted while the guard reported one.

        Edits do arrive legitimately. `main` never saw the freeze, so what it
        writes is history landing late, not history rewritten here, and
        dropping it would lose upstream work to a rule not aimed at it. What
        the freeze can still demand is that no drift be silent: the archive is
        compared by content, and anything that differs answers for itself in
        `FATE.md` — by record id for a decision, by path for the rest.
        """
        freeze = subprocess.run(
            ["git", "log", "--format=%H", "--grep", "move spec/ to docs/arch-v1",
             "--", "docs/arch-v1"],
            cwd=REPO_ROOT, capture_output=True, text=True, check=True,
        ).stdout.split()
        self.assertTrue(freeze, "the freeze commit must be findable")

        # Against the working tree, not against HEAD: an edit is a violation
        # the moment it exists, and a developer running this before committing
        # is exactly who should hear about it first.
        drifted = subprocess.run(
            ["git", "diff", "--name-only", freeze[0], "--", "docs/arch-v1"],
            cwd=REPO_ROOT, capture_output=True, text=True, check=True,
        ).stdout.split()

        # `FATE.md` is metadata about the archive rather than a record inside
        # it, and the surface ledger is generated output: freezing it would
        # freeze the generator.
        writable = {"docs/arch-v1/FATE.md", "docs/arch-v1/architecture/tool-surface.md"}
        fate = (REPO_ROOT / "docs" / "arch-v1" / "FATE.md").read_text(encoding="utf-8")

        # Only the arrivals section counts. `FATE.md` lists the fate of all
        # seventy-odd decisions by construction, so "the id appears somewhere
        # in the file" is true for every record and proves nothing about drift.
        section = fate.split(ARRIVALS_HEADING, 1)[1] if ARRIVALS_HEADING in fate else ""

        undisposed = []
        for path in sorted(set(drifted) - writable):
            relative = path[len("docs/arch-v1/"):]
            if relative not in section:
                undisposed.append(f"{relative}: differs from the freeze, arrivals section is silent")
        self.assertEqual(undisposed, [])


if __name__ == "__main__":
    unittest.main()
