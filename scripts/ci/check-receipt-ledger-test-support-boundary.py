#!/usr/bin/env python3
"""Признак `receipt-ledger-test-support` гейтит только элементы модуля.

Production-код рантайма v5 и хранилищ не ветвится по признаку тестовой
поддержки: атрибут `#[cfg(...)]`, упоминающий признак, может стоять только
перед элементом — `mod`, `use`, `fn`, `struct`, `enum`, `impl`, `trait`,
`type`, `const`, `static`, — а не перед оператором, выражением, аргументом,
полем или веткой `match`. Форма `not(feature = ...)` запрещена целиком: у
production не бывает кода, который есть только без признака.

Страж читает конструкции Cargo и языка, а не наши имена.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

FEATURE = "receipt-ledger-test-support"
SOURCE_ROOT = Path("crates/unica-coder/src")
CFG_ATTRIBUTE = re.compile(r"^\s*#\s*\[\s*cfg(?:_attr)?\s*\(")
NOT_FEATURE = re.compile(r"not\s*\(\s*feature\s*=\s*\"" + re.escape(FEATURE) + r"\"")
ATTRIBUTE_LINE = re.compile(r"^\s*#\s*\[")
ITEM_LINE = re.compile(
    r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?"
    r"(?:unsafe\s+|async\s+|const\s+|extern\s+(?:\"[^\"]*\"\s+)?)*"
    r"(?:mod|use|fn|struct|enum|impl|trait|type|const|static|macro_rules!)\b"
)


def gated_lines(source: str):
    lines = source.split("\n")
    for index, line in enumerate(lines):
        if not CFG_ATTRIBUTE.match(line) or FEATURE not in line:
            continue
        yield index, line, lines


def offenders(path: Path, source: str) -> list[str]:
    found = []
    for index, line, lines in gated_lines(source):
        if NOT_FEATURE.search(line):
            found.append(f"{path.as_posix()}:{index + 1}: `not(feature = ...)` is forbidden")
            continue
        # The attribute may span lines: skip to its closing bracket.
        cursor = index
        depth = line.count("[") - line.count("]")
        while depth > 0 and cursor + 1 < len(lines):
            cursor += 1
            depth += lines[cursor].count("[") - lines[cursor].count("]")
        # Skip further attributes and doc comments.
        target = cursor + 1
        while target < len(lines) and (
            ATTRIBUTE_LINE.match(lines[target]) or lines[target].strip().startswith("///")
        ):
            target += 1
        gated = lines[target] if target < len(lines) else ""
        if not ITEM_LINE.match(gated):
            found.append(
                f"{path.as_posix()}:{index + 1}: the feature gates `{gated.strip()[:60]}`, not an item"
            )
    return found


def scan(root: Path) -> list[str]:
    found = []
    for path in sorted((root / SOURCE_ROOT).rglob("*.rs")):
        found.extend(offenders(path.relative_to(root), path.read_text(encoding="utf-8")))
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    arguments = parser.parse_args()
    found = scan(arguments.root)
    for line in found:
        print(line)
    if found:
        print(f"{len(found)} feature-gated non-items", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
