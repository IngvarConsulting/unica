#!/usr/bin/env python3
"""Fail closed when an architecture-v1 subject has no explicit fate."""

from __future__ import annotations

import argparse
import re
import sys
from collections import Counter
from pathlib import Path


RULE_ID = re.compile(r"^### ((?:INV|REQ)-[A-Z]+(?:-[A-Z]+)+) — ", re.MULTILINE)
FATE_ROW = re.compile(
    r"^\|\s*`(?P<subject>[^`]+)`\s*\|\s*`(?P<fate>[^`]+)`\s*\|(?P<successor>.*?)\|\s*$"
)
V2_ID = re.compile(r"\b(?:DEC|INV|CTR)\.[A-Z0-9]+(?:[.-][A-Z0-9]+)+\b")
FRONTMATTER_ID = re.compile(r"^id:\s*((?:DEC|INV|CTR)\.[A-Z0-9.-]+)\s*$", re.MULTILINE)
ALLOWED_FATES = {"carried", "superseded", "retired"}


def v1_subjects(root: Path) -> set[str]:
    archive = root / "docs" / "arch-v1"
    subjects = {
        f"ADR-{record.name[:4]}"
        for record in (archive / "decisions").glob("[0-9][0-9][0-9][0-9]-*.md")
    }
    for registry in ("invariants.md", "quality-requirements.md"):
        source = archive / "architecture" / registry
        if source.is_file():
            subjects.update(RULE_ID.findall(source.read_text(encoding="utf-8")))
    subjects.update(
        f"acceptance/{contract.name}"
        for contract in (archive / "acceptance").glob("*.md")
    )
    return subjects


def v2_symbols(root: Path) -> set[str]:
    symbols: set[str] = set()
    for record in (root / "arch").rglob("*.md"):
        match = FRONTMATTER_ID.search(record.read_text(encoding="utf-8"))
        if match:
            symbols.add(match.group(1))
    return symbols


def fate_rows(root: Path) -> list[tuple[str, str, tuple[str, ...]]]:
    ledger = root / "docs" / "arch-v1" / "FATE.md"
    if not ledger.is_file():
        return []
    rows = []
    for line in ledger.read_text(encoding="utf-8").splitlines():
        match = FATE_ROW.match(line)
        if match:
            rows.append(
                (
                    match.group("subject"),
                    match.group("fate"),
                    tuple(V2_ID.findall(match.group("successor"))),
                )
            )
    return rows


def inspect(root: Path) -> list[str]:
    expected = v1_subjects(root)
    rows = fate_rows(root)
    known_v2 = v2_symbols(root)
    counts = Counter(subject for subject, _, _ in rows)
    errors: list[str] = []

    if not expected:
        errors.append("architecture-v1 archive has no ADR, INV, REQ, or acceptance subjects")
    for subject in sorted(expected - counts.keys()):
        errors.append(f"missing fate: {subject}")
    for subject in sorted(counts.keys() - expected):
        errors.append(f"unknown v1 subject in fate ledger: {subject}")
    for subject, count in sorted(counts.items()):
        if count != 1:
            errors.append(f"duplicate fate: {subject} appears {count} times")

    for subject, fate, successors in rows:
        if fate not in ALLOWED_FATES:
            errors.append(f"{subject}: unknown fate {fate!r}")
            continue
        if fate == "retired" and successors:
            errors.append(f"{subject}: retired subjects cannot name a successor")
        if fate in {"carried", "superseded"} and not successors:
            errors.append(f"{subject}: {fate} fate must name a v2 successor")
        for successor in successors:
            if successor not in known_v2:
                errors.append(f"{subject}: successor {successor} does not resolve")
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args(argv)
    root = args.root.resolve()
    errors = inspect(root)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"architecture-v1 fate coverage: {len(v1_subjects(root))} subjects")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
