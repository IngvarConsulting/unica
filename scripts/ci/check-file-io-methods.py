#!/usr/bin/env python3
"""Clippy pass that refuses filesystem methods in file-IO-free packages (#1894).

The text guard `check-rust-platform-boundary.py` sees `std::fs` paths and
explicit `Path::exists(path)` calls, but not `path.exists()`: that needs type
resolution. This pass runs Clippy's `disallowed_methods` with
`.config/clippy/file-io-free/clippy.toml` over `src/` of every package the
boundary names. `--all-targets --all-features` builds the library both as is
and with `cfg(test)`, and with every feature (`test-support` included);
only spans in `src/` count, so integration tests in `tests/` stay outside,
as INV.APP.DOMAIN-INPUTS allows.

First it proves the configuration on `scripts/ci/fixtures/file-io-probe`:
every disallowed method has one marked call there, and each must be refused.
A pass that refused nothing would otherwise look green forever.

Cargo runs Clippy on every workspace member it builds, dependencies included,
so the lint is a warning here and only spans inside `crates/<package>/src/`
of the checked packages count. Denying it would stop the build at a
dependency (workspace owns file IO legitimately) before the package itself.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
CONFIG = Path(".config/clippy/file-io-free")
PROBE = Path("scripts/ci/fixtures/file-io-probe")
LINT = "clippy::disallowed_methods"
MARKER = re.compile(r"//\s*probe:\s*(\S+)\s*$")


def disallowed(root: Path) -> list[str]:
    document = tomllib.loads((root / CONFIG / "clippy.toml").read_text(encoding="utf-8"))
    return [entry["path"] for entry in document["disallowed-methods"]]


def probe_markers(root: Path) -> dict[tuple[str, int], str]:
    markers = {}
    for path in sorted((root / PROBE / "src").rglob("*.rs")):
        relative = path.relative_to(root / PROBE).as_posix()
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            found = MARKER.search(line)
            if found:
                markers[(relative, number)] = found.group(1)
    return markers


def clippy_command(selection: list[str], target_dir: Path) -> list[str]:
    return ["cargo", "clippy", "--locked", *selection, "--all-targets", "--all-features",
            "--target-dir", str(target_dir), "--message-format=json",
            "--", "-A", "clippy::all", "-W", LINT]


def diagnostics(stdout: str) -> list[tuple[str, int, str]]:
    """(file, line, message) of every disallowed-method diagnostic."""
    found = []
    for line in stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        compiler = message.get("message") if message.get("reason") == "compiler-message" else None
        if not compiler or (compiler.get("code") or {}).get("code") != LINT:
            continue
        span = next((span for span in compiler.get("spans", []) if span.get("is_primary")), None)
        if span:
            found.append((span["file_name"], span["line_start"], compiler["message"]))
    return found


def run(command: list[str], cwd: Path, root: Path) -> subprocess.CompletedProcess:
    print("+ " + " ".join(command), flush=True)
    return subprocess.run(command, cwd=cwd, text=True, stdout=subprocess.PIPE,
                          env={**os.environ, "CLIPPY_CONF_DIR": str(root / CONFIG)})


def check_probe(root: Path, target_dir: Path) -> list[str]:
    methods, markers = disallowed(root), probe_markers(root)
    problems = [f"probe has no marked call for {method}" for method in methods if method not in markers.values()]
    problems += [f"probe marks {method}, which the configuration does not disallow"
                 for method in sorted(set(markers.values()) - set(methods))]
    completed = run(["cargo", "clippy", *clippy_command([], target_dir)[2:]], root / PROBE, root)
    refused = {(file, line): message for file, line, message in diagnostics(completed.stdout)}
    for (file, line), method in sorted(markers.items()):
        if f"`{method}`" not in refused.get((file, line), ""):
            problems.append(f"{file}:{line}: {method} was not refused")
    if completed.returncode != 0:
        problems.append(f"probe Clippy failed with exit code {completed.returncode}")
    return problems


def check_packages(root: Path, packages: dict[str, str], target_dir: Path) -> list[str]:
    """`packages` maps a package name to its repository directory."""
    if not packages:
        return ["no file-IO-free packages to check"]
    completed = run(clippy_command([argument for package in sorted(packages) for argument in ("-p", package)], target_dir), root, root)
    owned = tuple(f"{directory}/src/" for directory in packages.values())
    # The plain and the cfg(test) build of a library report the same span; keep it once.
    problems = sorted({f"{file}:{line}: {message}" for file, line, message in diagnostics(completed.stdout) if file.startswith(owned)})
    if completed.returncode != 0:
        problems.append(f"clippy failed with exit code {completed.returncode}")
    return problems


def boundary_packages(root: Path) -> dict[str, str]:
    spec = importlib.util.spec_from_file_location("rust_platform_boundary", Path(__file__).with_name("check-rust-platform-boundary.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.file_io_free_packages(root)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--repo-root", type=Path, default=ROOT)
    parser.add_argument("--target-dir", type=Path, default=None)
    args = parser.parse_args()
    root = args.repo_root.resolve()
    target_dir = (args.target_dir or root / "target").resolve()
    problems = check_probe(root, target_dir / "file-io-probe")
    if not problems:
        problems = check_packages(root, boundary_packages(root), target_dir)
    for problem in problems:
        print(f"::error::{problem}" if os.environ.get("GITHUB_ACTIONS") else problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
