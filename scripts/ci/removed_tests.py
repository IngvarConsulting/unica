#!/usr/bin/env python3
"""Find Rust tests a change removes and check the description declares them.

The queue gate compares each run with the previous inventory and refuses a
vanished test result unless a merged pull request description declares it
(`Removes-test:` or `Removes-test-target:`, #1816). A pull request run cannot
compare cases (its profile selects fewer tests), so a forgotten declaration
used to surface only in the queue and drop the whole batch. This check reads
`git diff` against the base, collects the `#[test]` functions that disappeared
from each crate and requires a declaration for each, before the queue:

    python scripts/ci/removed_tests.py --base origin/main --description-file pr.md

Without a build the full case ID is unknown, so the match is approximate:
the binary comes from `[[test]]`/`[[bin]]` of the package manifest or the
file name (`tests/x.rs` → `<package>::x`), the module path from the file
(`src/a/b.rs` → `<package>::a::b`); inline modules are not known. A
declaration of the test's name or a prefix ending in `*` counts when it agrees
with that path; `Removes-test-target:` counts for the file's binary. Packages
under `crates/` and `vendor/ureq` are checked. The queue still checks the exact
IDs.
"""
from __future__ import annotations

import argparse
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rust_inventory_baseline as baseline

ROOT = Path(__file__).resolve().parents[2]
TEST_FUNCTION = re.compile(
    r"#\[(?:[\w:]+::)?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*|//[^\n]*\n\s*)*"
    r"(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)"
)


def git(root: Path, *args: str) -> str:
    return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True).stdout


_spec = importlib.util.spec_from_file_location("rust_platform_boundary", Path(__file__).with_name("check-rust-platform-boundary.py"))
_boundary = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_boundary)


def package_root(path: str) -> str | None:
    """The Cargo package directory of a Rust file, as the queue lanes test it."""
    parts = PurePosixPath(path).parts
    if not path.endswith(".rs"):
        return None
    if len(parts) >= 3 and parts[0] == "crates":
        return f"crates/{parts[1]}"
    if path.startswith("vendor/ureq/"):
        return "vendor/ureq"
    return None


def tests_in(source: str) -> list[str]:
    # Comments and literals are not code: a `#[test]` in them is no test.
    return TEST_FUNCTION.findall(_boundary._mask_non_code(source))


class Packages:
    """Package names and binary IDs from `Cargo.toml` at one revision."""

    def __init__(self, root: Path, revision: str):
        self.root, self.revision, self.cache = root, revision, {}

    def manifest(self, directory: str) -> dict:
        if directory not in self.cache:
            try:
                self.cache[directory] = tomllib.loads(git(self.root, "show", f"{self.revision}:{directory}/Cargo.toml"))
            except subprocess.CalledProcessError:
                self.cache[directory] = {}
        return self.cache[directory]

    def name(self, directory: str) -> str:
        return self.manifest(directory).get("package", {}).get("name") or PurePosixPath(directory).name

    def case_prefix(self, path: str) -> tuple[str, str] | None:
        """(binary ID, module path) a file contributes to a case ID, when knowable."""
        directory = package_root(path)
        package, manifest = self.name(directory), self.manifest(directory)
        relative = PurePosixPath(path).relative_to(directory).as_posix()
        for kind, prefix in (("test", ""), ("bin", "bin/")):
            for target in manifest.get(kind, []):
                if target.get("path") == relative:
                    return f"{package}::{prefix}{target['name']}", ""
        parts = list(PurePosixPath(relative).parts)
        if parts[0] == "tests":
            # `tests/x.rs` is binary `crate::x`; files below `tests/` belong to a
            # binary that names them through `#[path]`, which is not known here.
            return (f"{package}::{PurePosixPath(parts[1]).stem}", "") if len(parts) == 2 else None
        if parts[0] != "src":
            return None
        if relative == "src/main.rs":
            return f"{package}::bin/{package}", ""
        if len(parts) == 3 and parts[1] == "bin":
            return f"{package}::bin/{PurePosixPath(parts[2]).stem}", ""
        modules = parts[1:]
        modules[-1] = PurePosixPath(modules[-1]).stem
        if modules[-1] in ("lib", "mod"):
            modules.pop()
        return package, "::".join(modules)


def removed_tests(root: Path, base: str, head: str = "HEAD") -> list[tuple[str, str, tuple[str, str] | None]]:
    """(package, test function, (binary, module) of its file) present at the merge base and gone at head."""
    merge_base = git(root, "merge-base", base, head).strip()
    changed = [line.split("\t") for line in git(root, "diff", "--name-status", "--no-renames", merge_base, head).splitlines()]
    paths = sorted({path for _, *names in changed for path in names if package_root(path)})
    packages = Packages(root, merge_base)
    before, after, files = Counter(), Counter(), {}
    for path in paths:
        package = packages.name(package_root(path))
        for revision, counter in ((merge_base, before), (head, after)):
            try:
                source = git(root, "show", f"{revision}:{path}")
            except subprocess.CalledProcessError:
                continue  # added or deleted on this side
            for name in tests_in(source):
                counter[(package, name)] += 1
                if counter is before:
                    files.setdefault((package, name), packages.case_prefix(path))
    # A test moved to another file of the package is not a removal.
    return sorted(((package, name, files[(package, name)]) for package, name in (before - after).elements()),
                  key=lambda item: item[:2])


def explained(package: str, name: str, known: tuple[str, str] | None, declarations: dict) -> bool:
    """A declaration names the test or a prefix that agrees with its file, or its binary."""
    case = None if known is None else (f"{known[0]}::{known[1]}::" if known[1] else f"{known[0]}::")
    for pattern in declarations["removedTests"]:
        if not pattern.startswith(package + "::"):
            continue
        prefix = pattern[:-1] if pattern.endswith("*") else pattern
        if not pattern.endswith("*") and not pattern.endswith("::" + name):
            continue
        # The file's module path is a prefix of the case ID; inline modules
        # may extend either side. Unknown paths are left to the queue.
        if case is None or case.startswith(prefix) or prefix.startswith(case):
            return True
    return any(target == known[0] if known else target == package or target.startswith(package + "::")
               for target in declarations["removedTargets"])


def declarations_in(description: str) -> dict:
    text = description.replace("\r\n", "\n")
    return {"removedTests": sorted(set(baseline.REMOVED_TEST.findall(text))),
            "removedTargets": sorted(set(baseline.REMOVED.findall(text)))}


def check(root: Path, base: str, description: str, head: str = "HEAD") -> list[str]:
    declarations = declarations_in(description)
    return [f"{crate}: test {name} disappeared; declare 'Removes-test: <binary-id>::<path>::{name}' "
            "(or a prefix ending in *) in the pull request description"
            for crate, name, known in removed_tests(root, base, head) if not explained(crate, name, known, declarations)]


def event_description() -> str | None:
    """The pull request description, read fresh: an edit after the event counts."""
    path = os.environ.get("GITHUB_EVENT_PATH")
    if os.environ.get("GITHUB_EVENT_NAME") != "pull_request" or not path:
        return None
    number = json.loads(Path(path).read_text(encoding="utf-8"))["pull_request"]["number"]
    return baseline.description(os.environ["GITHUB_REPOSITORY"], number) or ""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", required=True, help="base revision, e.g. origin/main or the pull request base SHA")
    parser.add_argument("--head", default="HEAD")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--description-file", type=Path, help="pull request description text")
    source.add_argument("--description", help="pull request description text")
    parser.add_argument("--repo-root", type=Path, default=ROOT)
    args = parser.parse_args()
    if args.description_file is not None:
        description = args.description_file.read_text(encoding="utf-8")
    elif args.description is not None:
        description = args.description
    else:
        description = event_description()
        if description is None:
            parser.error("give --description-file or --description outside a pull request run")
    problems = check(args.repo_root, args.base, description, args.head)
    for problem in problems:
        print(f"::error::{problem}" if os.environ.get("GITHUB_ACTIONS") else problem)
    if not problems:
        print("no undeclared removed tests")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
