#!/usr/bin/env python3.12
"""Fail a change that moves the public MCP surface without moving the spec layer.

The rule this guard enforces is INV-MCP-SURFACE-SYNC: adding, removing, or renaming a
public `unica.*` tool is an architecture change, so it lands together with the
decision record, registry entry, or acceptance plan that describes it.

The trigger is deliberately narrow. Only added or removed tool-name declarations
in the public registry count. Refactoring inside a handler, editing a schema
field, or renaming a local variable does not trip the guard, because a guard
that cries wolf is turned off within a week.

Usage:

    python3.12 scripts/ci/check-architecture-sync.py --base origin/main
    UNICA_DIFF_BASE=origin/main python3.12 scripts/ci/check-architecture-sync.py
    git diff origin/main... | python3.12 scripts/ci/check-architecture-sync.py -

Without a resolvable base the guard exits 0 and says so: a local checkout that
cannot name its base ref is not evidence of a violation.

That leniency is wrong in CI, where an unresolvable base means the job is
misconfigured, not that the change is clean. `--strict` turns every skip into a
failure, so a guard that cannot run reports itself instead of passing silently.
A shallow checkout is the usual cause: the three-dot diff needs a merge base,
which requires `fetch-depth: 0`.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

TOOL_REGISTRY = "crates/unica-coder/src/application/mod.rs"

# A public tool declaration in the registry, for example:  name: "unica.form.edit",
TOOL_DECLARATION = re.compile(r'name:\s*"(?P<tool>unica\.[A-Za-z0-9_.]+)"')

# Directories that carry the architecture contract. Touching any of them is
# accepted as evidence that the surface change was described.
ARCHITECTURE_PREFIXES = (
    "spec/decisions/",
    "spec/acceptance/",
    "spec/architecture/",
)

DIFF_FILE_HEADER = re.compile(r"^\+\+\+ b/(?P<path>.+)$")
# The old-side header is either `--- a/<path>` for a file that existed or
# `--- /dev/null` for one the diff creates. Matching both keeps the decision
# about existence in the code that cares, instead of leaving it implied by a
# pattern that silently drops half the grammar.
DIFF_OLD_HEADER = re.compile(r"^--- (?:a/)?(?P<path>.+)$")

# An accepted decision record is a dated statement of what was chosen, not a
# description of current code (INV-DOC-SUPERSEDE-NOT-EDIT). Two edits give the rewrite away and
# are cheap to spot in a diff: moving the acceptance date, and walking the status
# backwards. Prose edits stay legal, so translations and typo fixes pass.
DECISION_RECORD = re.compile(r"^spec/decisions/\d{4}-.+\.md$")
DATE_FIELD = re.compile(r"^-\s*(?:Дата|Date):\s*`?(?P<value>[0-9]{4}-[0-9]{2}-[0-9]{2})`?")
STATUS_FIELD = re.compile(r"^-\s*(?:Статус|Status):\s*`?(?P<value>[a-z]+)`?")
STATUS_ORDER = {"proposed": 0, "accepted": 1, "superseded": 2}


class SurfaceChange:
    def __init__(self) -> None:
        self.added: set[str] = set()
        self.removed: set[str] = set()
        self.architecture_files: set[str] = set()

    @property
    def touches_public_surface(self) -> bool:
        return bool(self.added or self.removed)

    @property
    def touches_architecture(self) -> bool:
        return bool(self.architecture_files)

    @property
    def is_violation(self) -> bool:
        return self.touches_public_surface and not self.touches_architecture

    def describe(self) -> str:
        lines = []
        if self.added:
            lines.append("  added tools:   " + ", ".join(sorted(self.added)))
        if self.removed:
            lines.append("  removed tools: " + ", ".join(sorted(self.removed)))
        if self.architecture_files:
            lines.append(
                "  architecture files touched: "
                + ", ".join(sorted(self.architecture_files))
            )
        return "\n".join(lines)


def analyze_decision_records(diff_text: str) -> list[str]:
    """Report accepted decision records that the diff rewrites (INV-DOC-SUPERSEDE-NOT-EDIT).

    Pure function over a unified diff: no git, no filesystem. A file that the
    diff creates is skipped -- a brand new record may say anything. Only edits
    to a record that already existed are judged, and only two of them: a moved
    acceptance date and a status that walks backwards.
    """
    violations: list[str] = []
    path: str | None = None
    # `--- a/<path>` marks a file as pre-existing; a created file carries
    # `--- /dev/null`. That line arrives before the `+++` line that names the
    # file, so the flag is parked in `pending_existed` and handed to the file
    # only when `+++` opens it. Without the hand-off a stale value from the
    # previous file would decide whether this one is judged.
    pending_existed = False
    existed = False
    dates: dict[str, list[str]] = {"-": [], "+": []}
    statuses: dict[str, list[str]] = {"-": [], "+": []}

    def close() -> None:
        if path is None or not existed:
            return
        if dates["-"] and dates["+"] and dates["-"] != dates["+"]:
            violations.append(
                f"{path}: acceptance date rewritten "
                f"({', '.join(dates['-'])} -> {', '.join(dates['+'])}); "
                "record the editorial change with an Updated field instead"
            )
        for before in statuses["-"]:
            for after in statuses["+"]:
                rank_before = STATUS_ORDER.get(before)
                rank_after = STATUS_ORDER.get(after)
                if rank_before is None or rank_after is None:
                    continue
                if rank_after < rank_before:
                    violations.append(
                        f"{path}: status moved backwards ({before} -> {after})"
                    )

    for line in diff_text.splitlines():
        new_header = DIFF_FILE_HEADER.match(line)
        if new_header:
            close()
            candidate = new_header.group("path")
            path = candidate if DECISION_RECORD.match(candidate) else None
            existed = pending_existed
            pending_existed = False
            dates = {"-": [], "+": []}
            statuses = {"-": [], "+": []}
            continue

        old_header = DIFF_OLD_HEADER.match(line)
        if old_header:
            pending_existed = old_header.group("path") != "/dev/null"
            continue

        if path is None or not line or line[0] not in "+-":
            continue
        if line.startswith(("+++", "---")):
            continue

        side, body = line[0], line[1:]
        date = DATE_FIELD.match(body)
        if date:
            dates[side].append(date.group("value"))
        status = STATUS_FIELD.match(body)
        if status:
            statuses[side].append(status.group("value"))

    close()
    return violations


def analyze_diff(diff_text: str) -> SurfaceChange:
    """Classify a unified diff. Pure function: no git, no filesystem."""
    change = SurfaceChange()
    current_path: str | None = None

    for line in diff_text.splitlines():
        new_header = DIFF_FILE_HEADER.match(line)
        if new_header:
            path = new_header.group("path")
            current_path = None if path == "/dev/null" else path
            if current_path and current_path.startswith(ARCHITECTURE_PREFIXES):
                change.architecture_files.add(current_path)
            continue

        old_header = DIFF_OLD_HEADER.match(line)
        if old_header:
            path = old_header.group("path")
            # A deleted file has no +++ path, so record the old path too.
            if path != "/dev/null" and path.startswith(ARCHITECTURE_PREFIXES):
                change.architecture_files.add(path)
            continue

        if current_path != TOOL_REGISTRY:
            continue

        if line.startswith("+++") or line.startswith("---"):
            continue

        if line.startswith("+"):
            for match in TOOL_DECLARATION.finditer(line[1:]):
                change.added.add(match.group("tool"))
        elif line.startswith("-"):
            for match in TOOL_DECLARATION.finditer(line[1:]):
                change.removed.add(match.group("tool"))

    # A pure rename inside one hunk shows the same tool on both sides only when
    # the name really changed, so nothing to cancel out here. A moved line with
    # an unchanged name does appear on both sides; drop that noise.
    unchanged = change.added & change.removed
    change.added -= unchanged
    change.removed -= unchanged
    return change


def resolve_base(explicit: str | None) -> str | None:
    for candidate in (explicit, os.environ.get("UNICA_DIFF_BASE")):
        if candidate:
            return candidate
    for candidate in ("origin/main", "main"):
        result = subprocess.run(
            ["git", "rev-parse", "--verify", "--quiet", candidate],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return candidate
    return None


def read_diff(base: str) -> str | None:
    result = subprocess.run(
        ["git", "diff", "--unified=0", f"{base}..."],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return None
    return result.stdout


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="git ref the change is measured against")
    parser.add_argument(
        "--strict",
        action="store_true",
        help="treat an unusable base ref as a failure instead of a skip",
    )
    parser.add_argument(
        "diff",
        nargs="?",
        help="read a unified diff from stdin when set to '-'",
    )
    args = parser.parse_args(argv)

    def unusable(message: str) -> int:
        if args.strict:
            print(f"check-architecture-sync: {message}")
            print(
                "--strict was requested, so this is a failure: the guard could "
                "not run and therefore proves nothing. A shallow checkout is "
                "the usual cause; the three-dot diff needs `fetch-depth: 0`."
            )
            return 2
        print(f"check-architecture-sync: {message}; skipping")
        return 0

    if args.diff == "-":
        diff_text = sys.stdin.read()
    else:
        base = resolve_base(args.base)
        if base is None:
            return unusable(
                "no base ref resolved (pass --base or set UNICA_DIFF_BASE)"
            )
        diff_text = read_diff(base)
        if diff_text is None:
            return unusable(f"cannot diff against {base!r}")

    rewritten = analyze_decision_records(diff_text)
    if rewritten:
        print("check-architecture-sync: an accepted decision record was rewritten")
        for violation in rewritten:
            print(f"  {violation}")
        print()
        print(
            "INV-DOC-SUPERSEDE-NOT-EDIT: a record states what was chosen on its date. When the\n"
            "choice stops applying, supersede it with a new record instead of\n"
            "editing it to match the code."
        )
        return 1

    change = analyze_diff(diff_text)
    if not change.touches_public_surface:
        print("check-architecture-sync: public MCP surface unchanged")
        return 0

    if not change.is_violation:
        print("check-architecture-sync: surface change is described")
        print(change.describe())
        return 0

    print("check-architecture-sync: public MCP surface changed without spec sync")
    print(change.describe())
    print()
    print(
        "INV-MCP-SURFACE-SYNC requires the owning decision record, the registry entry that\n"
        "derives from it, and the check named by that entry to change together.\n"
        "Update one of: spec/decisions/, spec/architecture/, spec/acceptance/."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
