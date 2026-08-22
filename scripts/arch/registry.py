#!/usr/bin/env python3
"""Read the architecture v2 registry and render its index.

Three registries share one record shape: a markdown file that opens with a
front-matter block of props and continues with prose. The symbol and the path
derive from each other, so navigation never needs the index — the index exists
so a reader can hold the whole registry in one screen and grep it in one pass.

The front-matter parser is a deliberate subset of YAML: scalars, flat lists and
`null`. A registry record that needs more structure than that is a record that
outgrew its purpose.

Usage:
    registry.py                 # печатает индекс в stdout
    registry.py --write-index   # записывает arch/index.md
    registry.py --check         # молча выходит с 1, если индекс устарел
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ARCH_ROOT = REPO_ROOT / "arch"
INDEX_PATH = ARCH_ROOT / "index.md"

KIND_BY_DIR = {"decisions": "decision", "invariants": "invariant", "contracts": "contract"}
SYMBOL_PREFIX = {"decision": "DEC", "invariant": "INV", "contract": "CTR"}

REQUIRED_PROPS = {
    "decision": ("id", "status", "governs", "realized"),
    "invariant": ("id", "status", "governs", "decision", "check", "scope"),
    "contract": (
        "id",
        "status",
        "governs",
        "version",
        "decision",
        "producer",
        "consumers",
        "check",
        "scope",
    ),
}

# Кто заметит нарушение: потребитель или только мы. Ось решает не предмет
# записи, а адресат обещания, и от неё зависит, чем правка оплачивается.
GOVERNS = ("product", "process")

FRONT_MATTER = re.compile(r"\A---\n(.*?)\n---\n(.*)\Z", re.S)
DECISION_FILENAME = re.compile(r"\A(\d{4}-\d{2}-\d{2})-([a-z0-9-]+)\.md\Z")
DECISION_SYMBOL = re.compile(r"\ADEC\.(\d{4}-\d{2}-\d{2})\.([A-Z0-9-]+)\Z")
SYMBOL_ANYWHERE = re.compile(r"\b(?:DEC\.\d{4}-\d{2}-\d{2}|INV|CTR)\.[A-Z0-9.-]+\b")

# A symbol becomes a filename, and Windows still refuses these as base names
# whatever the extension follows. `CON` was the first contract prefix and made
# the whole tree impossible to check out on Windows.
DOS_DEVICE_NAMES = frozenset(
    ["CON", "PRN", "AUX", "NUL"]
    + [f"COM{digit}" for digit in range(10)]
    + [f"LPT{digit}" for digit in range(10)]
)


@dataclass
class Record:
    id: str
    kind: str
    path: Path
    props: dict = field(default_factory=dict)
    body: str = ""

    @property
    def summary(self) -> str:
        """The first heading, or the first non-empty line without markup."""
        for line in self.body.splitlines():
            line = line.strip()
            if not line:
                continue
            return line.lstrip("# ").strip()
        return ""

    @property
    def relative(self) -> str:
        return self.path.relative_to(ARCH_ROOT).as_posix()


def parse_front_matter(text: str) -> tuple[dict, str]:
    """Split a record into its props and its body.

    Supports `key: scalar`, `key: [a, b]` and `key: null`. Anything else is a
    parse error rather than a silent partial read.
    """
    match = FRONT_MATTER.match(text)
    if not match:
        raise ValueError("record does not open with a front-matter block")
    props: dict = {}
    for number, line in enumerate(match.group(1).splitlines(), start=1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if ":" not in line:
            raise ValueError(f"front matter line {number} is not `key: value`: {line!r}")
        key, _, raw = line.partition(":")
        key, raw = key.strip(), raw.strip()
        if raw.startswith("[") and raw.endswith("]"):
            inner = raw[1:-1].strip()
            props[key] = [item.strip() for item in inner.split(",") if item.strip()]
        elif raw in ("null", "~", ""):
            props[key] = None
        else:
            props[key] = raw
    return props, match.group(2)


def records(root: Path = ARCH_ROOT) -> list[Record]:
    """Every record of every registry, ordered by symbol."""
    found: list[Record] = []
    for directory, kind in KIND_BY_DIR.items():
        base = root / directory
        if not base.is_dir():
            continue
        for path in sorted(base.glob("*.md")):
            props, body = parse_front_matter(path.read_text(encoding="utf-8"))
            identifier = props.get("id") or ""
            found.append(Record(id=identifier, kind=kind, path=path, props=props, body=body))
    return sorted(found, key=lambda record: record.id)


def expected_symbol(record: Record) -> str:
    """The symbol a record's own path demands."""
    if record.kind == "decision":
        match = DECISION_FILENAME.match(record.path.name)
        if not match:
            return ""
        date, slug = match.groups()
        return f"DEC.{date}.{slug.upper()}"
    return record.path.stem


def validation_errors(found: list[Record]) -> list[str]:
    """Return violations of the published record schema."""
    by_id = {record.id: record for record in found}
    errors: list[str] = []
    for record in found:
        for key in REQUIRED_PROPS[record.kind]:
            realized_is_planned = (
                record.kind == "decision"
                and key == "realized"
                and record.props.get("status") == "planned"
            )
            if (
                key not in record.props
                or record.props[key] == ""
                or (record.props[key] is None and not realized_is_planned)
            ):
                errors.append(f"{record.relative}: missing prop `{key}`")
        if record.kind in {"invariant", "contract"}:
            list_keys = ("scope",) + (("consumers",) if record.kind == "contract" else ())
            for key in list_keys:
                if not isinstance(record.props.get(key), list) or not record.props[key]:
                    errors.append(f"{record.relative}: `{key}` must be a non-empty list")
            owner = by_id.get(record.props.get("decision"))
            if owner is None or owner.kind != "decision":
                errors.append(f"{record.relative}: decision does not resolve to a decision")
            elif record.props.get("status") == "active" and owner.props.get("status") != "active":
                errors.append(f"{record.relative}: active rule cites a non-active decision")
            elif record.id not in (owner.props.get("establishes") or []):
                errors.append(
                    f"{owner.relative}: does not establish its rule {record.id}"
                )
        if record.kind == "contract":
            version = str(record.props.get("version", ""))
            if not version.isdecimal() or int(version) < 1:
                errors.append(f"{record.relative}: version must be a positive integer")
        if record.kind == "decision":
            if "changes" in record.props:
                changed_contracts = record.props.get("changes")
                if not isinstance(changed_contracts, list) or not changed_contracts:
                    errors.append(
                        f"{record.relative}: `changes` must be a list and not empty"
                    )
                else:
                    for contract_id in changed_contracts:
                        contract = by_id.get(contract_id)
                        if contract is None:
                            errors.append(
                                f"{record.relative}: changes cites missing contract "
                                f"{contract_id}"
                            )
                        elif contract.kind != "contract":
                            errors.append(
                                f"{record.relative}: changes cites non-contract "
                                f"{contract_id}"
                            )
            for rule_id in record.props.get("establishes") or []:
                rule = by_id.get(rule_id)
                if rule is not None and rule.props.get("decision") != record.id:
                    errors.append(
                        f"{record.relative}: establishes a rule owned by "
                        f"{rule.props.get('decision')}: {rule_id}"
                    )
    return errors


def render_index(found: list[Record]) -> str:
    """One line per symbol, sorted, with no fact that props do not carry."""
    kind_ru = {"decision": "решение", "invariant": "инвариант", "contract": "контракт"}
    lines = [
        "<!-- ПОРОЖДАЕТСЯ scripts/arch/registry.py --write-index; руками не правится -->",
        "",
        "# Индекс реестра",
        "",
        "| Символ | Вид | Статус | Построено | Суть | Файл |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for record in found:
        # Построено отвечает только за решения: у инварианта и контракта
        # свидетельство обязательно по схеме, а у решения — необязательно, и
        # именно там читатель не отличает принятое от сделанного.
        built = ""
        if record.kind == "decision":
            built = "нет" if record.props.get("realized") in (None, "") else "да"
        lines.append(
            f"| `{record.id}` | {kind_ru[record.kind]} · {record.props.get('governs', '')} "
            f"| {record.props.get('status', '')} "
            f"| {built} | {record.summary} | [{record.relative}]({record.relative}) |"
        )
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--write-index", action="store_true")
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args(argv)

    found = records()
    errors = validation_errors(found)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1

    rendered = render_index(found)
    if arguments.write_index:
        INDEX_PATH.write_text(rendered, encoding="utf-8")
        print(f"написано: {INDEX_PATH.relative_to(REPO_ROOT)}")
        return 0
    if arguments.check:
        current = INDEX_PATH.read_text(encoding="utf-8") if INDEX_PATH.is_file() else ""
        if current != rendered:
            print("индекс устарел: перегенерируйте --write-index", file=sys.stderr)
            return 1
        return 0
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
