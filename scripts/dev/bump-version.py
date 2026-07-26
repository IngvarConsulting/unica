#!/usr/bin/env python3
"""Set the Unica release version everywhere the package contract declares it.

The version appears in several files because each one is read by a different
consumer: Cargo compiles it into the binaries, the two host manifests ship it to
Codex and Claude Code, and the tools lock pins it alongside the third-party
tools. They are separate artifacts, so the version cannot live in one file, but
it can be written by one command and verified by one check.

`scripts/ci/check-version-contract.py` is the gate; this script is the way to
satisfy it without editing files by hand.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


SEMVER = re.compile(r"^\d+\.\d+\.\d+$")


def render_cargo_version(original: str, version: str, path: Path) -> str:
    # Only the workspace package version, never a dependency's version field.
    updated, count = re.subn(
        r'(?m)^(\[workspace\.package\](?:\n(?!\[).*)*?\nversion = ")[^"]+(")',
        rf"\g<1>{version}\g<2>",
        original,
        count=1,
    )
    if count != 1:
        raise SystemExit(f"could not locate workspace.package.version in {path}")
    return updated


def render_json_version(original: str, version: str, path: Path) -> str:
    data = json.loads(original)
    data["version"] = version
    return json.dumps(data, ensure_ascii=False, indent=2) + "\n"


def render_tools_lock_version(original: str, version: str, path: Path) -> str:
    data = json.loads(original)
    entries = [tool for tool in data.get("tools", []) if tool.get("name") == "unica"]
    if len(entries) != 1:
        raise SystemExit(f"expected exactly one unica entry in {path}, found {len(entries)}")
    entries[0]["version"] = version
    return json.dumps(data, ensure_ascii=False, indent=2) + "\n"


def bump(repo_root: Path, version: str) -> list[str]:
    """Render every file first, then write.

    A malformed file part-way through the list would otherwise leave the
    repository straddling two versions, which is the exact state the version
    contract exists to forbid. Rendering everything before touching disk keeps a
    failure a no-op.
    """
    plugin = repo_root / "plugins" / "unica"
    targets = [
        (repo_root / "Cargo.toml", render_cargo_version),
        (plugin / ".codex-plugin" / "plugin.json", render_json_version),
        (plugin / ".claude-plugin" / "plugin.json", render_json_version),
        (plugin / "third-party" / "tools.lock.json", render_tools_lock_version),
    ]

    pending = []
    for path, render in targets:
        if not path.is_file():
            # A host manifest may legitimately not exist yet on older branches.
            continue
        original = path.read_text(encoding="utf-8")
        updated = render(original, version, path)
        if original != updated:
            pending.append((path, updated))

    for path, updated in pending:
        path.write_text(updated, encoding="utf-8")
    return [path.relative_to(repo_root).as_posix() for path, _ in pending]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("version", help="release version without the leading v, for example 0.9.2")
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    args = parser.parse_args()

    version = args.version.removeprefix("v")
    if not SEMVER.fullmatch(version):
        raise SystemExit(f"version must be MAJOR.MINOR.PATCH, got {args.version}")

    repo_root = args.repo_root.resolve()
    changed = bump(repo_root, version)
    for path in changed:
        print(f"updated {path}")
    if not changed:
        print(f"already at {version}")

    print("refresh Cargo.lock with: cargo update --workspace --offline")
    contract = subprocess.run(
        [sys.executable, "scripts/ci/check-version-contract.py", "--expected", version],
        cwd=repo_root,
        check=False,
    )
    return contract.returncode


if __name__ == "__main__":
    raise SystemExit(main())
