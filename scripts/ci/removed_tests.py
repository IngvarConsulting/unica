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

The case ID is rebuilt without a build, as nextest names it: the binary from
the crate targets the queue lanes build (`[[test]]`/`[[bin]]` of the manifest
whose `required-features` a lane enables, `src/lib.rs`, `tests/x.rs` →
`<package>::x`), the module path from the module tree (`mod`,
`#[path]`, `include!` and inline modules) and the function name. A declaration
counts by the queue gate's own rule: the exact ID or a literal prefix ending
in `*` of the same binary, or `Removes-test-target:` for a binary that is
gone. Packages under `crates/` and `vendor/ureq` are checked. `cfg` is not
evaluated; tests a macro generates and files reached only through a computed
`include!` are not seen. The queue still checks what this misses.
"""
from __future__ import annotations

import argparse
from collections import Counter
import importlib.util
import json
import os
import posixpath
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ci_rust_lanes
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


def lane_features() -> dict[str, set[str]]:
    """Features a queue lane enables, by package name (`-p`) or manifest
    directory (`--manifest-path`): a target whose `required-features` they
    lack is never built, so its tests are no case of the inventory."""
    enabled: dict[str, set[str]] = {}
    for lane in ci_rust_lanes.LANES:
        selected = list(ci_rust_lanes.selection(lane))
        owners = [selected[i + 1] for i, flag in enumerate(selected) if flag == "-p"]
        owners += [str(PurePosixPath(selected[i + 1]).parent) for i, flag in enumerate(selected) if flag == "--manifest-path"]
        features = {feature for i, flag in enumerate(selected) if flag == "--features"
                    for feature in selected[i + 1].split(",")}
        for owner in owners:
            enabled.setdefault(owner, set()).update(features)
    return enabled


LANE_FEATURES = lane_features()


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


def tests_in(source: str) -> list[tuple[str, str]]:
    """(inline module path, name) of each test function in a file."""
    # Comments and literals are not code: a `#[test]` in them is no test.
    masked = _boundary._mask_non_code(source)
    events = sorted([(item.start(), "test", item[1]) for item in TEST_FUNCTION.finditer(masked)]
                    + [(item.start(), "item", item) for item in MODULE_ITEM.finditer(masked)],
                    key=lambda event: event[0])
    found, inline, depth = [], [], 0
    for _, kind, item in events:
        if kind == "test":
            found.append(("::".join(name for _, name in inline), item))
        elif item["brace"] == "{" or item["end"] == "{":
            depth += 1
            if item["end"] == "{":
                inline.append((depth, item["name"]))
        elif item["brace"] == "}":
            if inline and inline[-1][0] == depth:
                inline.pop()
            depth -= 1
    return found


# A module item as it reads with comments and literals masked: attributes,
# visibility and `mod name;` or `mod name {`; an `include!(...)`; or a brace
# that opens or closes some other block. An attribute holds no bracket once
# literals are masked, so the repetition has one way to match.
MODULE_ITEM = re.compile(
    r"(?P<attributes>(?:#\[[^\[\]]*\]\s*)*)"
    r"(?:pub(?:\([^)]*\))?\s+)?\bmod\s+(?P<name>\w+)\s*(?P<end>[;{])"
    r"|\binclude!\s*\((?P<include>[^)]*)\)|(?P<brace>[{}])")
PATH_ATTRIBUTE = re.compile(r"#\[path\s*=([^\[\]]*)\]")
STRING = re.compile(r'^\s*"([^"\\]*)"\s*$')


class Packages:
    """Package names and case ID prefixes of files from `Cargo.toml` and the
    module tree at one revision."""

    def __init__(self, root: Path, revision: str):
        self.root, self.revision, self.cache, self.trees, self.sources = root, revision, {}, {}, {}

    def source(self, path: str) -> str | None:
        """A file's text at the revision; one `git cat-file` reads a package."""
        directory = package_root(path)
        if directory not in self.sources:
            listing = git(self.root, "ls-tree", "-r", self.revision, "--", directory).splitlines()
            entries = [line.split("\t", 1) for line in listing]
            blobs = [(name, meta.split()[2]) for meta, name in entries if meta.split()[1] == "blob" and name.endswith(".rs")]
            output = subprocess.run(["git", "cat-file", "--batch"], cwd=self.root, check=True, capture_output=True,
                                    input="".join(f"{blob}\n" for _, blob in blobs).encode()).stdout
            texts, offset = {}, 0
            for name, _ in blobs:
                header_end = output.index(b"\n", offset)
                size = int(output[offset:header_end].split()[2])
                texts[name] = output[header_end + 1:header_end + 1 + size].decode("utf-8", "replace")
                offset = header_end + 2 + size
            self.sources[directory] = texts
        return self.sources[directory].get(path)

    def manifest(self, directory: str) -> dict:
        if directory not in self.cache:
            try:
                self.cache[directory] = tomllib.loads(git(self.root, "show", f"{self.revision}:{directory}/Cargo.toml"))
            except subprocess.CalledProcessError:
                self.cache[directory] = {}
        return self.cache[directory]

    def name(self, directory: str) -> str:
        return self.manifest(directory).get("package", {}).get("name") or PurePosixPath(directory).name

    def targets(self, directory: str) -> list[tuple[str, str]]:
        """(binary ID, root file) of each crate target, as Cargo discovers them."""
        package, manifest = self.name(directory), self.manifest(directory)
        self.source(f"{directory}/src/lib.rs")
        files = set(self.sources[directory])
        roots = {}
        for kind, prefix in (("test", ""), ("bin", "bin/")):
            for target in manifest.get(kind, []):
                # A target whose file is gone builds nothing: Cargo fails, and
                # the declaration of its removal is what the queue needs.
                features = LANE_FEATURES.get(package, set()) | LANE_FEATURES.get(directory, set())
                if (target.get("path") and f"{directory}/{target['path']}" in files
                        and set(target.get("required-features", [])) <= features):
                    roots[f"{directory}/{target['path']}"] = f"{package}::{prefix}{target['name']}"
        # A manifest target owns its file: Cargo discovers it no second time.
        declared = {f"{directory}/{target['path']}" for kind in ("test", "bin")
                    for target in manifest.get(kind, []) if target.get("path")}
        def discover(path, binary):
            if path in files and path not in declared:
                roots.setdefault(path, binary)
        discover(f"{directory}/src/lib.rs", package)
        discover(f"{directory}/src/main.rs", f"{package}::bin/{package}")
        for path in sorted(files):
            parts = PurePosixPath(path).relative_to(directory).parts
            if len(parts) == 3 and parts[:2] == ("src", "bin") and path.endswith(".rs"):
                discover(path, f"{package}::bin/{PurePosixPath(parts[2]).stem}")
            elif len(parts) == 4 and parts[:2] == ("src", "bin") and parts[3] == "main.rs":
                discover(path, f"{package}::bin/{parts[2]}")
            elif len(parts) == 2 and parts[0] == "tests" and path.endswith(".rs"):
                discover(path, f"{package}::{PurePosixPath(parts[1]).stem}")
            elif len(parts) == 3 and parts[0] == "tests" and parts[2] == "main.rs":
                discover(path, f"{package}::{parts[1]}")
        return sorted((binary, path) for path, binary in roots.items())

    def tree(self, directory: str) -> dict[str, set[tuple[str, str]]]:
        """Each file reached from a target by `mod`, `#[path]` or `include!`,
        with every (binary ID, module path) it is compiled as."""
        if directory in self.trees:
            return self.trees[directory]
        reached: dict[str, set[tuple[str, str]]] = {}
        pending = [(path, binary, "", True) for binary, path in self.targets(directory)]
        while pending:
            path, binary, module, mod_rs = pending.pop()
            if (binary, module) in reached.get(path, set()):
                continue
            source = self.source(path)
            if source is None:
                continue
            reached.setdefault(path, set()).add((binary, module))
            pending.extend(self.children(path, source, binary, module, mod_rs))
        self.trees[directory] = reached
        return reached

    @staticmethod
    def children(path: str, source: str, binary: str, module: str, mod_rs: bool):
        """Files a source declares, with the module each is compiled as."""
        masked = _boundary._mask_non_code(source)
        here = PurePosixPath(path).parent
        own = here if mod_rs else here / PurePosixPath(path).stem
        inline: list[tuple[int, str]] = []
        depth = 0
        def joined(*names):
            return "::".join(name for name in (module, *names) if name)
        for item in MODULE_ITEM.finditer(masked):
            if item["brace"] == "{":
                depth += 1
            elif item["brace"] == "}":
                if inline and inline[-1][0] == depth:
                    inline.pop()
                depth -= 1
            elif item["include"] is not None:
                literal = STRING.match(source[item.start("include"):item.end("include")])
                if literal:
                    yield (posixpath.normpath(str(here / literal[1])), binary,
                           joined(*(name for _, name in inline)), mod_rs)
            elif item["end"] == "{":
                depth += 1
                inline.append((depth, item["name"]))
            else:
                names = [name for _, name in inline]
                path = PATH_ATTRIBUTE.search(masked, item.start("attributes"), item.end("attributes"))
                if path:
                    literal = STRING.match(source[path.start(1):path.end(1)])
                    if literal:
                        base = (here if mod_rs or not names else own).joinpath(*names)
                        yield posixpath.normpath(str(base / literal[1])), binary, joined(*names, item["name"]), True
                    continue
                base = own.joinpath(*names)
                yield str(base / f"{item['name']}.rs"), binary, joined(*names, item["name"]), False
                yield str(base / item["name"] / "mod.rs"), binary, joined(*names, item["name"]), True

    def case_prefixes(self, path: str) -> list[tuple[str, str]]:
        """Every (binary ID, module path) a file contributes to case IDs; none
        when no crate target reaches it."""
        return sorted(self.tree(package_root(path)).get(path, ()))


def cases(packages: Packages, directories: list[str]) -> dict[str, tuple[str, list[str]]]:
    """Case ID of every test the packages compile, with its package and the
    binary IDs of the package, as the queue inventory names them."""
    found = {}
    for directory in directories:
        package = packages.name(directory)
        binaries = [binary for binary, _ in packages.targets(directory)]
        for path, prefixes in packages.tree(directory).items():
            source = packages.source(path)
            for inline, name in tests_in(source):
                for binary, module in prefixes:
                    found["::".join(part for part in (binary, module, inline, name) if part)] = (package, binaries)
    return found


def removed_tests(root: Path, base: str, head: str = "HEAD") -> list[tuple[str, str, list[str], list[str]]]:
    """(package, case ID, binary IDs of the package at the base and those gone
    at head) of each test compiled at the merge base and not at head, for the
    packages the change touches."""
    merge_base = git(root, "merge-base", base, head).strip()
    changed = [line.split("\t") for line in git(root, "diff", "--name-status", "--no-renames", merge_base, head).splitlines()]
    directories = sorted({package_root(path) for _, *names in changed for path in names if package_root(path)})
    before = cases(Packages(root, merge_base), directories)
    after = cases(Packages(root, head), directories)
    remaining = {binary for _, binaries in after.values() for binary in binaries}
    remaining |= {binary for directory in directories for binary, _ in Packages(root, head).targets(directory)}
    # A test moved to another module changes its case ID: the queue sees it go.
    return sorted((package, case, binaries, sorted(set(binaries) - remaining))
                  for case, (package, binaries) in before.items() if case not in after)


# The queue gate (`ci_rust_lanes.explained_case`) matches a declaration
# literally; only a trailing `*` makes it a prefix. Braces, `?`, brackets or an
# inner `*` would read as a pattern here and as a literal in the queue.
UNSUPPORTED_PATTERN = re.compile(r"[{}?\[\]]|\*.")


def unsupported_declarations(declarations: dict) -> list[str]:
    return [f"declaration '{kind}: {item}' uses a pattern the queue does not understand; "
            "only a trailing * is a prefix, list each name otherwise"
            for kind, key in (("Removes-test", "removedTests"), ("Removes-test-target", "removedTargets"))
            for item in declarations[key] if UNSUPPORTED_PATTERN.search(item)]


def binary_of(case: str, binaries: list[str]) -> str | None:
    # As `ci_rust_lanes.binary_of`: a case belongs to the longest binary ID it starts with.
    return max((binary for binary in binaries if case.startswith(binary + "::")), key=len, default=None)


def explained(case: str, binaries: list[str], gone: list[str], declarations: dict) -> bool:
    """The queue gate's rule (`ci_rust_lanes.explained_case`): the case's binary
    is gone and declared removed, or a declaration of the same binary names the
    case exactly or, ending in `*`, is a literal prefix of it."""
    binary = binary_of(case, binaries)
    if binary in gone and binary in declarations["removedTargets"]:
        return True
    return any(binary_of(pattern, binaries) == binary
               and (case.startswith(pattern[:-1]) if pattern.endswith("*") else case == pattern)
               for pattern in declarations["removedTests"])


def declarations_in(description: str) -> dict:
    text = description.replace("\r\n", "\n")
    return {"removedTests": sorted(set(baseline.REMOVED_TEST.findall(text))),
            "removedTargets": sorted(set(baseline.REMOVED.findall(text)))}


def check(root: Path, base: str, description: str, head: str = "HEAD") -> list[str]:
    declarations = declarations_in(description)
    return unsupported_declarations(declarations) + [
        f"{package}: test {case} disappeared; declare 'Removes-test: {case}' "
        "(or a literal prefix ending in *) in the pull request description"
        for package, case, binaries, gone in removed_tests(root, base, head)
        if not explained(case, binaries, gone, declarations)]


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
