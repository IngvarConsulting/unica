#!/usr/bin/env python3
"""Fail closed when an architecture-v1 subject has no explicit fate."""

from __future__ import annotations

import argparse
import re
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path


RULE_ID = re.compile(r"^### ((?:INV|REQ)-[A-Z]+(?:-[A-Z]+)+) — ", re.MULTILINE)
FATE_ROW = re.compile(
    r"^\|\s*`(?P<subject>[^`]+)`\s*\|\s*`(?P<fate>[^`]+)`\s*\|"
    r"(?P<successor>.*?)\|(?P<reason>.*?)\|\s*$"
)
FRONTMATTER_ID = re.compile(r"^id:\s*((?:DEC|INV|CTR)\.[A-Z0-9.-]+)\s*$", re.MULTILINE)
FRONT_MATTER = re.compile(r"\A---\s*\n(?P<props>.*?)\n---(?:\s*\n|\Z)", re.DOTALL)
INLINE_CODE = re.compile(r"`([^`]+)`")
LITERAL_TOOL_NAME = re.compile(r"`unica\.[^`\s]+`")
SUCCESSOR_TOKEN = re.compile(
    r"`(?P<id>(?:DEC|INV|CTR)\.[A-Z0-9]+(?:[.-][A-Z0-9]+)+)`"
)
SUCCESSOR_SEPARATOR = re.compile(r"\s*(?:,\s*(?:<br>\s*)?|<br>\s*)\Z")
BEHAVIOR_REMOVED = re.compile(
    r"^behavior-removed:\s+(?P<decision>DEC\.[A-Z0-9]+(?:[.-][A-Z0-9]+)+)$"
)
ALLOWED_FATES = {"carried", "superseded", "retired"}
RETIRED_REASONS = {"tool-surface-bound", "check-removed", "historical-only"}
PROCESS_SUBJECTS = {
    "INV-APP-DISPATCH-OWNERSHIP",
    "INV-APP-THIN-TRANSPORT",
    "INV-APP-NO-ADAPTER-BYPASS",
    "INV-APP-NO-SCRIPT-BACKEND",
    "INV-APP-DEPENDENCY-DIRECTION",
    "INV-APP-NO-DIRECT-GIT",
    "INV-MCP-DATA-DRIVEN-SCHEMA",
    "INV-MCP-SDK-TRANSPORT",
}
PRODUCT_SUBJECTS = {
    "INV-APP-CODE-PROVIDER-BOUNDARY",
    "INV-APP-DOCUMENTATION-NETWORK-POLICY",
    "INV-APP-SUPPORT-STATE",
    "INV-APP-PARTIAL-FALLBACK",
    "INV-APP-CONFIG-SNAPSHOT",
    "INV-APP-DIAGNOSTIC-PROVIDERS",
    "INV-APP-DOCUMENTATION-NO-DISK-STATE",
    "INV-APP-LAZY-HIDDEN-SERVICES",
    "INV-APP-OUTLINE-SOURCE",
}
PRODUCT_PREFIXES = (
    "INV-PRODUCT-",
    "INV-MCP-",
    "INV-CACHE-",
    "INV-SOURCE-",
    "INV-PKG-",
    "REQ-PERF-",
    "REQ-TOKEN-",
    "REQ-SAFETY-",
    "REQ-OBS-",
    "REQ-COMPAT-",
    "REQ-REL-",
)
PROCESS_PREFIXES = ("INV-CI-", "INV-DOC-", "REQ-MAINT-")


@dataclass(frozen=True)
class FateRow:
    subject: str
    fate: str
    successor_cell: str
    successors: tuple[str, ...]
    reason: str


@dataclass(frozen=True)
class V1Rule:
    body: str
    checks: tuple[str, ...]


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


def _cell_value(cell: str) -> str:
    value = cell.strip()
    if len(value) >= 2 and value.startswith("`") and value.endswith("`"):
        return value[1:-1]
    return value


def fate_rows(root: Path) -> list[FateRow]:
    ledger = root / "docs" / "arch-v1" / "FATE.md"
    if not ledger.is_file():
        return []
    rows = []
    for line in ledger.read_text(encoding="utf-8").splitlines():
        match = FATE_ROW.match(line)
        if match:
            successor_cell = match.group("successor").strip()
            rows.append(
                FateRow(
                    subject=match.group("subject"),
                    fate=match.group("fate"),
                    successor_cell=successor_cell,
                    successors=tuple(
                        token.group("id") for token in SUCCESSOR_TOKEN.finditer(successor_cell)
                    ),
                    reason=_cell_value(match.group("reason")),
                )
            )
    return rows


def _valid_successor_list(cell: str) -> bool:
    tokens = list(SUCCESSOR_TOKEN.finditer(cell))
    if not tokens or cell[: tokens[0].start()].strip():
        return False
    for previous, current in zip(tokens, tokens[1:]):
        if not SUCCESSOR_SEPARATOR.fullmatch(cell[previous.end() : current.start()]):
            return False
    return not cell[tokens[-1].end() :].strip()


def v1_rules(root: Path) -> dict[str, V1Rule]:
    archive = root / "docs" / "arch-v1" / "architecture"
    rules: dict[str, V1Rule] = {}
    for registry in ("invariants.md", "quality-requirements.md"):
        source = archive / registry
        if not source.is_file():
            continue
        text = source.read_text(encoding="utf-8")
        matches = list(RULE_ID.finditer(text))
        for index, match in enumerate(matches):
            end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
            body = text[match.start() : end]
            checks = []
            for line in body.splitlines():
                if "**Check:**" not in line:
                    continue
                inline_values = INLINE_CODE.findall(line)
                if len(inline_values) >= 2:
                    checks.append(inline_values[-1])
            rules[match.group(1)] = V1Rule(body=body, checks=tuple(checks))
    return rules


def _front_matter_props(path: Path) -> dict[str, str]:
    match = FRONT_MATTER.match(path.read_text(encoding="utf-8"))
    if not match:
        return {}
    props: dict[str, str] = {}
    for line in match.group("props").splitlines():
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        props[key.strip()] = value.strip()
    return props


def v2_decisions(root: Path) -> dict[str, dict[str, str]]:
    decisions: dict[str, dict[str, str]] = {}
    for path in (root / "arch" / "decisions").glob("*.md"):
        props = _front_matter_props(path)
        identifier = props.get("id", "")
        if identifier.startswith("DEC."):
            decisions[identifier] = props
    return decisions


def _check_resolves(root: Path, check: str) -> bool:
    relative, separator, name = check.partition("::")
    path = root / relative
    if not path.is_file():
        return False
    if not separator:
        return True
    return name in path.read_text(encoding="utf-8")


def _legacy_governs(subject: str) -> str | None:
    if subject in PROCESS_SUBJECTS:
        return "process"
    if subject in PRODUCT_SUBJECTS:
        return "product"
    if subject.startswith(PRODUCT_PREFIXES):
        return "product"
    if subject.startswith(PROCESS_PREFIXES):
        return "process"
    return None


def _retirement_errors(
    root: Path,
    row: FateRow,
    rules: dict[str, V1Rule],
    decisions: dict[str, dict[str, str]],
) -> list[str]:
    reason = row.reason
    if reason in ("", "—"):
        return [f"{row.subject}: retired fate requires a reason"]

    behavior = BEHAVIOR_REMOVED.fullmatch(reason)
    if behavior:
        decision_id = behavior.group("decision")
        decision = decisions.get(decision_id)
        if decision is None:
            return [f"{row.subject}: behavior-removal decision {decision_id} does not resolve"]
        if decision.get("status") != "active":
            return [
                f"{row.subject}: behavior-removal decision {decision_id} has status "
                f"{decision.get('status')!r}, not 'active'"
            ]
        expected_governs = _legacy_governs(row.subject)
        if expected_governs is None:
            return [
                f"{row.subject}: cannot classify legacy behavior as product or process; "
                f"behavior-removal decision {decision_id} is not admissible"
            ]
        actual_governs = decision.get("governs")
        if actual_governs != expected_governs:
            return [
                f"{row.subject}: behavior-removal decision {decision_id} governs mismatch: "
                f"expected {expected_governs!r}, got {actual_governs!r}"
            ]
        return []

    if reason not in RETIRED_REASONS:
        return [f"{row.subject}: unknown retirement reason {reason!r}"]
    if reason == "historical-only":
        if not (row.subject.startswith("ADR-") or row.subject.startswith("acceptance/")):
            return [f"{row.subject}: historical-only is limited to ADR and acceptance subjects"]
        return []

    rule = rules.get(row.subject)
    if rule is None:
        return [f"{row.subject}: {reason} requires an old INV/REQ rule block"]
    if reason == "tool-surface-bound":
        if not LITERAL_TOOL_NAME.search(rule.body):
            return [
                f"{row.subject}: tool-surface-bound requires a literal `unica.*` "
                "identity in the old rule"
            ]
        return []

    if not rule.checks:
        return [f"{row.subject}: check-removed requires at least one old check"]
    resolving = [check for check in rule.checks if _check_resolves(root, check)]
    if resolving:
        return [
            f"{row.subject}: check-removed is false because checks still resolve: "
            + ", ".join(resolving)
        ]
    return []


def inspect(root: Path) -> list[str]:
    expected = v1_subjects(root)
    rows = fate_rows(root)
    known_v2 = v2_symbols(root)
    rules = v1_rules(root)
    decisions = v2_decisions(root)
    counts = Counter(row.subject for row in rows)
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

    for row in rows:
        if row.fate not in ALLOWED_FATES:
            errors.append(f"{row.subject}: unknown fate {row.fate!r}")
            continue
        if row.fate == "retired":
            if row.successor_cell != "—":
                errors.append(
                    f"{row.subject}: retired successor cell must be exactly '—', got "
                    f"{row.successor_cell!r}"
                )
            errors.extend(_retirement_errors(root, row, rules, decisions))
        else:
            if not _valid_successor_list(row.successor_cell):
                errors.append(
                    f"{row.subject}: {row.fate} successor cell must contain only backtick-quoted "
                    "v2 IDs separated by comma and/or <br>, got "
                    f"{row.successor_cell!r}"
                )
            if row.reason != "—":
                errors.append(f"{row.subject}: {row.fate} fate must use reason '—'")
        for successor in row.successors:
            if successor not in known_v2:
                errors.append(f"{row.subject}: successor {successor} does not resolve")
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
