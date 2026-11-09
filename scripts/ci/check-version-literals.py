#!/usr/bin/env python3
"""Refuse new version literals in Rust production code (INV.SOURCE.PROFILE-CAPABILITIES).

Code asks the source profile what exists and what is allowed; only the profile,
the format family crates and their dialects know which format and platform
versions there are. A version literal elsewhere is a place a new version would
have to edit: a comparison with `"2.20"`, `PLATFORM_XML_8_3_27_FORMAT_2_20`,
`version="2.20"`, a `*_8_3_27` function, an `ACTIVE_*` profile constant.

Each crate declares in its `Cargo.toml`:

    [package.metadata.unica]
    version-literal-baseline = 12   # literals its production code already has

A crate fails when its count exceeds its baseline, so existing literals stay
and new ones do not. The key lives in the crate, so PRs touching different
crates do not meet in one list. A lowered count does not fail;
`--write-baseline` lowers the baselines to the current counts.

Production code is a crate's `.rs` files outside `tests/`, `*_tests.rs`,
`tests.rs`, test-support and fixture files, cut at the first `#[cfg(test)]`
module; comment lines are skipped. The files that own versions are allowed.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
BASELINE_KEY = "version-literal-baseline"

# Classes A–E of the multi-version plan: profile identity, address grammar id,
# root version and written version literals, version-named catalogs.
LITERAL = re.compile(
    r'"2\.(?:1\d|2\d)"'           # "2.20"
    r"|\b2_(?:1\d|2\d)\b"          # 2_20
    r'|"8\.[35]\.\d+"'             # "8.3.27", "8.5.1"
    r"|8_[35]_\d+"                 # 8_3_27 in names: v8_3_27, *_8_3_27, Version8_3_27
    r'|version=\\?"2\.\d+'         # version="2.20" in written XML
    r"|FORMAT_2_\d+|Format2_\d+"   # PLATFORM_XML_8_3_27_FORMAT_2_20, Format2_20
    r"|\bACTIVE_(?:SERIALIZATION_FORMAT|PLATFORM_LINE|FORMAT_PROFILE)\b"
)
# The places that own versions: the format identity and the platform line,
# the Designer format family's table, and format dialects.
OWNERS = (
    "crates/unica-source-contract/src/format_identity.rs",
    "crates/unica-source-designer/src/format_profile.rs",
)
OWNER_DIRECTORIES = ("/dialect/",)
TEST_MODULE = re.compile(r"\n\s*#\[cfg\(test\)\]\s*\n\s*(?:pub(?:\([^)]*\))?\s+)?mod\s")


@dataclass
class Crate:
    name: str
    directory: str
    baseline: int
    manifest: Path


@dataclass
class Literal:
    path: str
    line: int
    text: str


def load_crates(root: Path) -> list[Crate]:
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    members = sorted(
        directory.relative_to(root)
        for pattern in workspace["workspace"]["members"]
        for directory in root.glob(pattern)
    )
    crates = []
    for member in members:
        manifest = root / member / "Cargo.toml"
        document = tomllib.loads(manifest.read_text(encoding="utf-8"))
        settings = document["package"].get("metadata", {}).get("unica", {})
        crates.append(
            Crate(
                name=document["package"]["name"],
                directory=Path(member).as_posix(),
                baseline=int(settings.get(BASELINE_KEY, 0)),
                manifest=manifest,
            )
        )
    return crates


def is_production(relative: str) -> bool:
    name = relative.rsplit("/", 1)[-1]
    if "/tests/" in relative or "/target/" in relative or "/benches/" in relative:
        return False
    if name == "tests.rs" or name.endswith("_tests.rs") or name == "build.rs":
        return False
    return "test_support" not in name and "fixture" not in name


def is_owner(relative: str) -> bool:
    return relative in OWNERS or any(part in relative for part in OWNER_DIRECTORIES)


def production_text(source: str) -> str:
    """The text before the first test module."""
    match = TEST_MODULE.search(source)
    return source[: match.start()] if match else source


def source_literals(relative: str, source: str) -> list[Literal]:
    found = []
    for number, line in enumerate(production_text(source).splitlines(), start=1):
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        for match in LITERAL.finditer(line):
            found.append(Literal(relative, number, match.group(0)))
    return found


def crate_literals(root: Path, crate: Crate) -> list[Literal]:
    found = []
    for path in sorted((root / crate.directory).rglob("*.rs")):
        relative = path.relative_to(root).as_posix()
        if not is_production(relative) or is_owner(relative):
            continue
        found.extend(source_literals(relative, path.read_text(encoding="utf-8", errors="replace")))
    return found


def check(root: Path = ROOT) -> list[str]:
    errors = []
    for crate in load_crates(root):
        literals = crate_literals(root, crate)
        if len(literals) > crate.baseline:
            errors.append(
                f"{crate.name}: {len(literals)} version literals in production code, "
                f"{BASELINE_KEY} is {crate.baseline}; ask the source profile or the format "
                "family instead of naming a version (INV.SOURCE.PROFILE-CAPABILITIES); "
                f"`{Path(__file__).name} --report` lists them"
            )
    return errors


def report(root: Path = ROOT) -> str:
    lines = []
    for crate in load_crates(root):
        literals = crate_literals(root, crate)
        if literals:
            lines.append(f"{crate.name}: {len(literals)} (baseline {crate.baseline})")
            lines.extend(f"  {item.path}:{item.line}: {item.text}" for item in literals)
    return "\n".join(lines)


def with_baseline(text: str, count: int) -> str:
    """Manifest text with the baseline set; a zero baseline is the absent key."""
    newline = "\r\n" if "\r\n" in text else "\n"
    lines = text.split(newline)
    key = next((i for i, line in enumerate(lines) if line.startswith(BASELINE_KEY)), None)
    entry = f"{BASELINE_KEY} = {count}"
    if key is not None:
        if count:
            lines[key] = entry
        else:
            del lines[key]
        return newline.join(lines)
    if not count:
        return text
    section = next((i for i, line in enumerate(lines) if line.strip() == "[package.metadata.unica]"), None)
    if section is not None:
        lines.insert(section + 1, entry)
        return newline.join(lines)
    body = text.rstrip("\r\n")
    return f"{body}{newline}{newline}[package.metadata.unica]{newline}{entry}{newline}"


def write_baseline(root: Path = ROOT) -> None:
    for crate in load_crates(root):
        count = len(crate_literals(root, crate))
        with crate.manifest.open(encoding="utf-8", newline="") as stream:
            text = stream.read()
        updated = with_baseline(text, count)
        if updated != text:
            with crate.manifest.open("w", encoding="utf-8", newline="") as stream:
                stream.write(updated)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--report", action="store_true", help="print the literals of every crate")
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help=f"set {BASELINE_KEY} of every crate to its current count",
    )
    arguments = parser.parse_args()
    if arguments.write_baseline:
        write_baseline(arguments.root)
    if arguments.report:
        print(report(arguments.root))
    errors = check(arguments.root)
    for error in errors:
        print(error, file=sys.stderr)
    print(f"version literals: {len(errors)} errors")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
