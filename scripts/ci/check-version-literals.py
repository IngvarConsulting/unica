#!/usr/bin/env python3
"""Refuse new version literals in Rust production code (INV.SOURCE.PROFILE-CAPABILITIES).

Code asks the source profile what exists and what is allowed; only the profile,
the format family crates and their dialects know which format and platform
versions there are. A version literal elsewhere is a place a new version would
have to edit: a comparison with `"2.20"`, `PLATFORM_XML_8_3_27_FORMAT_2_20`,
`version="2.20"`, a `*_8_3_27` function, an `ACTIVE_*` profile constant, a
platform line value `PlatformLine::new(8, 5, 1)`.

Each crate declares in its `Cargo.toml`:

    [package.metadata.unica]
    version-literal-baseline = 12   # literals its production code already has

A crate fails when its count exceeds its baseline, so existing literals stay
and new ones do not. The key lives in the crate, so PRs touching different
crates do not meet in one list. A lowered count does not fail;
`--write-baseline` lowers the baselines to the current counts.

Production code is a crate's `.rs` files whose path within the crate has no
test part: `tests`, `*_tests`, test support or fixtures, as a directory or a
file name, so the submodules in `foo_tests/` are test code as `foo_tests.rs` is.
A file gated whole by a leading `#![cfg(test)]` is test code too. Each file is
cut at its first `#[cfg(test)]` module; comment lines are
skipped. The files that own versions are allowed. Literals are matched line by
line, so a `PlatformLine::new` call whose arguments rustfmt wrapped is not seen;
a comparison of two line values names no version and is not seen either.
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
    r"|PlatformLine::new\(\s*8\s*,\s*\d+\s*,\s*\d+\s*\)"  # PlatformLine::new(8, 5, 1)
)
# The places that own versions: the format identity and the platform line,
# the Designer format family's table, and format dialects.
OWNERS = (
    "crates/unica-source-contract/src/format_identity.rs",
    "crates/unica-source-designer/src/format_profile.rs",
)
OWNER_DIRECTORIES = ("/dialect/",)
TEST_MODULE = re.compile(r"\n\s*#\[cfg\(test\)\]\s*\n\s*(?:pub(?:\([^)]*\))?\s+)?mod\s")
# A file whose first item is `#![cfg(test)]`, after comments, compiles only in tests.
TEST_FILE = re.compile(r"\A(?:\s*//[^\n]*\n)*\s*#!\[cfg\(test\)\]")


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


def is_test_part(part: str) -> bool:
    """A directory or file stem that holds test code: `tests`, `foo_tests`, test support, fixtures."""
    return (
        part in ("tests", "benches", "target")
        or part.endswith("_tests")
        or "test_support" in part
        or "fixture" in part
    )


def is_production(inside: str) -> bool:
    """`inside` is the path within the crate. A test module's submodules live in its
    directory, so every directory on the path counts, not only the file name."""
    *directories, name = inside.split("/")
    if name == "build.rs":
        return False
    return not any(is_test_part(part) for part in [*directories, name.removesuffix(".rs")])


def is_owner(relative: str) -> bool:
    return relative in OWNERS or any(part in relative for part in OWNER_DIRECTORIES)


def production_text(source: str) -> str:
    """The text before the first test module; nothing for a file gated whole by `#![cfg(test)]`."""
    if TEST_FILE.match(source):
        return ""
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
    directory = root / crate.directory
    for path in sorted(directory.rglob("*.rs")):
        relative = path.relative_to(root).as_posix()
        if not is_production(path.relative_to(directory).as_posix()) or is_owner(relative):
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
