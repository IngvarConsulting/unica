#!/usr/bin/env python3.12
"""Fail a change that moves the public MCP surface without moving the spec layer.

The rule this guard enforces is INV-MCP-08: adding, removing, or renaming a
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
DIFF_OLD_HEADER = re.compile(r"^--- a/(?P<path>.+)$")


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
        "INV-MCP-08 requires the owning decision record, the registry entry that\n"
        "derives from it, and the check named by that entry to change together.\n"
        "Update one of: spec/decisions/, spec/architecture/, spec/acceptance/."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
