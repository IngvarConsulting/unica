#!/usr/bin/env python3
"""Refuse new Rust reads of files outside the crate that reads them.

Choosing tests by changed crates holds only while a crate sees nothing but its
own files and its declared inputs. A read outside it is one of:

- a `tests/fixtures` path of the repository;
- a `../` path from `CARGO_MANIFEST_DIR` or in `include_str!`, `include_bytes!`
  and `#[path]`, when it resolves outside the crate;
- a walk up from `CARGO_MANIFEST_DIR` by `.parent()`, `.ancestors()` or `..`.

Each crate declares in its `Cargo.toml`:

    [package.metadata.unica]
    inputs = ["tests/fixtures/shared"]   # repository paths the crate may read
    read-outside-baseline = 12           # undeclared reads it already has

A read under a declared input is allowed. A crate fails when its undeclared
reads exceed its baseline, so existing ones stay and new ones do not. Both keys
live in the crate, so PRs touching different crates do not meet in one list.

The guard reads source text and does not follow values: a path returned by a
helper is counted where the helper walks up, not where the path is used.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import posixpath
import re
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parents[2]
MANIFEST_VARIABLE = "CARGO_MANIFEST_DIR"
FIXTURES = "tests/fixtures"
FIXTURES_PATH = re.compile(r"(?:^|/)tests/fixtures(?:/|$)")
# A source without one of these has nothing to report, and most sources have none.
CANDIDATE = re.compile(r"\.\./|tests/fixtures|CARGO_MANIFEST_DIR")
TOKEN = re.compile(
    r"""
    //[^\n]*
    | /\*.*?\*/
    | (?<![A-Za-z0-9_])b?r(?P<hashes>\#*)"(?P<raw>.*?)"(?P=hashes)
    | (?<![A-Za-z0-9_])b?"(?P<plain>(?:\\.|[^"\\])*)"
    | '(?:\\.|[^'\\\n])'
    """,
    re.DOTALL | re.VERBOSE,
)
FILE_RELATIVE = re.compile(r"(?:include_str!|include_bytes!)\s*\(\s*$|#\[path\s*=\s*$")
FILE_RELATIVE_REACH = 64
CFG_TEST = re.compile(r"#\[cfg\((?:all\(|any\()?test\b")
WALK_UP = re.compile(r"\.parent\(\)|\.ancestors\(\)")
STATEMENT_END = re.compile(r"[;{}]")
TEST_FILE = re.compile(r"(?:^|/)(?:tests?|benches)(?:/|\.rs$)|_tests?(?:/|\.rs$)|(?:^|/)test_[^/]*\.rs$")


@dataclass(frozen=True)
class Read:
    crate: str
    file: str
    line: int
    target: str
    kind: str


@dataclass
class Crate:
    name: str
    directory: str
    inputs: list[str]
    baseline: int
    manifest: Path


def blank(text: str) -> str:
    return re.sub(r"[^\n]", " ", text)


def scan_tokens(source: str) -> tuple[str, list[tuple[int, str]]]:
    """Source with comments and string bodies blanked, and the string values."""
    parts: list[str] = []
    strings: list[tuple[int, str]] = []
    position = 0
    for match in TOKEN.finditer(source):
        parts.append(source[position:match.start()])
        text = match.group()
        value = match.group("raw") if match.group("raw") is not None else match.group("plain")
        if value is None:
            parts.append(blank(text))
        else:
            strings.append((match.start(), value))
            parts.append('"' + blank(text[2:]) + '"' if len(text) >= 2 else text)
        position = match.end()
    parts.append(source[position:])
    return "".join(parts), strings


def statement_bounds(code: str, offset: int) -> tuple[int, int]:
    start = max(code.rfind(mark, 0, offset) for mark in ";{}") + 1
    end = STATEMENT_END.search(code, offset)
    return start, end.start() if end else len(code)


def kind_of(relative: str, code: str, offset: int) -> str:
    inside_crate = relative.split("/", 2)[2]
    if inside_crate == "build.rs":
        return "build"
    if TEST_FILE.search(inside_crate):
        return "test"
    marker = CFG_TEST.search(code)
    return "test" if marker and marker.start() < offset else "production"


def outside_target(crate: Crate, base: str, value: str) -> str | None:
    """Repository path of `value` from `base`, or None while it stays in the crate."""
    target = posixpath.normpath(posixpath.join(base, value.lstrip("/")))
    if target == crate.directory or target.startswith(crate.directory + "/"):
        return None
    return target


def fixture_target(crate: Crate, value: str, root: Path) -> str | None:
    """Repository path named by a fixture literal that has no known base."""
    match = FIXTURES_PATH.search(value)
    assert match is not None
    start = match.start() + (1 if value[match.start()] == "/" else 0)
    prefix = value[:start].replace("../", "").strip("./")
    tail = value[start:]
    if prefix == crate.directory:
        return None
    if not prefix and (root / crate.directory / tail).exists():
        return None
    return f"{prefix}/{tail}" if prefix.startswith("crates/") else tail


def source_reads(crate: Crate, relative: str, source: str, root: Path) -> list[Read]:
    if not CANDIDATE.search(source):
        return []
    code, strings = scan_tokens(source)
    reads: list[Read] = []
    walked: set[int] = set()

    def report(offset: int, target: str) -> None:
        line = source.count("\n", 0, offset) + 1
        reads.append(Read(crate.name, relative, line, target, kind_of(relative, code, offset)))

    for offset, raw_value in strings:
        value = raw_value.replace("\\\\", "/").replace("\\", "/")
        if not (value == MANIFEST_VARIABLE or ".." in value or FIXTURES in value):
            continue
        start, end = statement_bounds(code, offset)
        uses_manifest = MANIFEST_VARIABLE in source[start:end]
        walks_up = uses_manifest and (
            WALK_UP.search(code[start:end]) is not None
            or any(v == ".." for o, v in strings if start <= o < end)
        )
        if value == MANIFEST_VARIABLE:
            # A walk that names its fixture is counted by that literal, which an
            # input can declare; a bare walk has only the repository as its target.
            names_fixture = any(
                FIXTURES_PATH.search(v) for o, v in strings if start <= o < end
            )
            if walks_up and not names_fixture and start not in walked:
                walked.add(start)
                report(offset, ".")
            continue
        has_parent = ".." in value.split("/")
        has_fixture = FIXTURES_PATH.search(value) is not None
        if not (has_parent or has_fixture) or value == "..":
            continue
        if FILE_RELATIVE.search(code[max(0, offset - FILE_RELATIVE_REACH) : offset]):
            directory = posixpath.dirname(relative)
            target = outside_target(crate, directory, value)
        elif uses_manifest and not walks_up:
            target = outside_target(crate, crate.directory, value)
        elif has_fixture and walks_up:
            target = value[FIXTURES_PATH.search(value).start():].lstrip("/")
        elif has_fixture:
            target = fixture_target(crate, value, root)
        else:
            target = None
        if target is not None:
            report(offset, target)
    return reads


def load_crates(root: Path) -> list[Crate]:
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    crates = []
    for member in workspace["workspace"]["members"]:
        manifest = root / member / "Cargo.toml"
        document = tomllib.loads(manifest.read_text(encoding="utf-8"))
        settings = document["package"].get("metadata", {}).get("unica", {})
        crates.append(
            Crate(
                name=document["package"]["name"],
                directory=Path(member).as_posix(),
                inputs=list(settings.get("inputs", [])),
                baseline=int(settings.get("read-outside-baseline", 0)),
                manifest=manifest,
            )
        )
    return crates


def crate_sources(root: Path, crate: Crate):
    directory = root / crate.directory
    for path in sorted(directory.rglob("*.rs")):
        relative = path.relative_to(root).as_posix()
        inside = relative[len(crate.directory) + 1 :]
        # A crate's own fixtures may hold Rust text that is data, not its code.
        if inside.startswith("tests/fixtures/") or "/target/" in relative:
            continue
        yield relative, path.read_text(encoding="utf-8", errors="replace")


def crate_reads(root: Path, crate: Crate) -> list[Read]:
    reads: list[Read] = []
    for relative, source in crate_sources(root, crate):
        reads.extend(source_reads(crate, relative, source, root))
    return reads


def declared(target: str, inputs: list[str]) -> bool:
    return any(target == item or target.startswith(item.rstrip("/") + "/") for item in inputs)


def input_errors(root: Path, crate: Crate) -> list[str]:
    errors = []
    for item in crate.inputs:
        normal = posixpath.normpath(item)
        if normal in {".", ""} or normal.startswith("..") or posixpath.isabs(item) or "\\" in item:
            errors.append(f"{crate.name}: input `{item}` must be a repository path below the root")
        elif not (root / normal).exists():
            errors.append(f"{crate.name}: input `{item}` does not exist")
    return errors


def undeclared_reads(root: Path, crate: Crate) -> list[Read]:
    return [read for read in crate_reads(root, crate) if not declared(read.target, crate.inputs)]


def check(root: Path = ROOT) -> list[str]:
    errors: list[str] = []
    for crate in load_crates(root):
        errors.extend(input_errors(root, crate))
        reads = undeclared_reads(root, crate)
        if len(reads) > crate.baseline:
            errors.append(
                f"{crate.name}: {len(reads)} reads outside the crate, baseline "
                f"{crate.baseline}. Move the file into the crate or declare it in "
                "[package.metadata.unica] inputs of its Cargo.toml:"
            )
            errors.extend(f"  {read.file}:{read.line} -> {read.target}" for read in reads)
    return errors


def report(root: Path = ROOT) -> str:
    rows = []
    for crate in load_crates(root):
        reads = undeclared_reads(root, crate)
        if reads or crate.baseline:
            kinds = [sum(read.kind == kind for read in reads) for kind in ("test", "production", "build")]
            rows.append((crate.name, len(reads), *kinds, crate.baseline))
    rows.sort(key=lambda row: (-row[1], row[0]))
    width = max([len(row[0]) for row in rows] + [len("crate")])
    lines = [f"{'crate':<{width}}  reads  test  production  build  baseline"]
    lines.extend(
        f"{row[0]:<{width}}  {row[1]:>5}  {row[2]:>4}  {row[3]:>10}  {row[4]:>5}  {row[5]:>8}"
        for row in rows
    )
    totals = [sum(row[index] for row in rows) for index in range(1, 6)]
    lines.append(
        f"{'total':<{width}}  {totals[0]:>5}  {totals[1]:>4}  {totals[2]:>10}  {totals[3]:>5}  {totals[4]:>8}"
    )
    return "\n".join(lines)


def with_baseline(text: str, count: int) -> str:
    """Manifest text with the baseline set; a zero baseline is the absent key."""
    newline = "\r\n" if "\r\n" in text else "\n"
    lines = text.split(newline)
    key = next((i for i, line in enumerate(lines) if line.startswith("read-outside-baseline")), None)
    entry = f"read-outside-baseline = {count}"
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
        count = len(undeclared_reads(root, crate))
        with crate.manifest.open(encoding="utf-8", newline="") as stream:
            text = stream.read()
        updated = with_baseline(text, count)
        if updated != text:
            with crate.manifest.open("w", encoding="utf-8", newline="") as stream:
                stream.write(updated)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--report", action="store_true", help="print the reads of every crate")
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help="set read-outside-baseline of every crate to its current count",
    )
    arguments = parser.parse_args()
    started = time.monotonic()
    if arguments.write_baseline:
        write_baseline(arguments.root)
    if arguments.report:
        print(report(arguments.root))
    errors = check(arguments.root)
    for error in errors:
        print(error, file=sys.stderr)
    print(f"crate read boundary: {len(errors)} errors, {time.monotonic() - started:.2f} s")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
