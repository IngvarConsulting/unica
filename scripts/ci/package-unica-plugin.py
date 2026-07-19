#!/usr/bin/env python3
"""Assemble a Codex marketplace package from built Unica tool artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import tarfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path


PLUGIN_ID = "unica"
DISPLAY_NAME = "Unica"
SOURCE_PACKAGE_IGNORES = {"bin", ".DS_Store", "__pycache__", ".pytest_cache"}
DISALLOWED_ARCHIVE_PARTS = {".build", "dist", "__pycache__", ".pytest_cache"}


def copytree(src: Path, dst: Path, *, ignore: set[str] | None = None) -> None:
    ignore = ignore or set()
    if dst.exists():
        shutil.rmtree(dst)

    def _ignore(_dir: str, names: list[str]) -> set[str]:
        return set(names) & ignore

    shutil.copytree(src, dst, ignore=_ignore)


def copy_binary_tree(src: Path, dst: Path) -> None:
    copytree(src, dst)
    for path in dst.rglob("*"):
        if path.is_file():
            path.chmod(path.stat().st_mode | 0o111)


def git_tracked_plugin_files(repo_root: Path, plugin_src: Path) -> list[str]:
    rel_plugin = plugin_src.relative_to(repo_root).as_posix()
    result = subprocess.run(
        ["git", "-C", str(repo_root), "ls-files", "-z", "--", rel_plugin],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    prefix = f"{rel_plugin}/"
    return [
        path[len(prefix) :]
        for path in result.stdout.decode("utf-8").split("\0")
        if path.startswith(prefix)
    ]


def validate_tracked_plugin_source_path(rel_path: Path) -> None:
    if rel_path.is_absolute() or ".." in rel_path.parts:
        raise SystemExit(f"git tracked plugin file escapes plugin root: {rel_path.as_posix()}")
    ignored_parts = set(rel_path.parts) & SOURCE_PACKAGE_IGNORES
    if ignored_parts:
        raise SystemExit(
            f"source package path is generated or ignored: {rel_path.as_posix()}"
        )


def copy_tracked_plugin_source(repo_root: Path, plugin_src: Path, dst: Path) -> None:
    if dst.exists():
        shutil.rmtree(dst)
    dst.mkdir(parents=True)

    for rel in git_tracked_plugin_files(repo_root, plugin_src):
        rel_path = Path(rel)
        validate_tracked_plugin_source_path(rel_path)
        source = plugin_src / rel_path
        if source.is_symlink():
            raise SystemExit(f"tracked plugin source symlink is not allowed: {rel_path.as_posix()}")
        target = dst / rel_path
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def load_lock(path: Path) -> dict:
    lock = json.loads(path.read_text(encoding="utf-8"))
    if lock.get("schemaVersion") != 1:
        raise SystemExit(f"unsupported tools lock schemaVersion in {path}: {lock.get('schemaVersion')}")
    return lock


def lock_by_tool(lock: dict) -> dict[str, dict]:
    return {tool["name"]: tool for tool in lock.get("tools", [])}


def validate_tool_against_lock(tool: dict, locked: dict, target: str) -> None:
    checks = {
        "version": "version",
        "repository": "repository",
        "sourceTag": "sourceTag",
        "sourceCommit": "sourceCommit",
        "license": "license",
    }
    for actual_key, lock_key in checks.items():
        if tool[actual_key] != locked[lock_key]:
            raise SystemExit(
                f"{tool['name']} {actual_key} differs from lock: {tool[actual_key]} != {locked[lock_key]}"
            )

    if target not in locked.get("assets", {}):
        raise SystemExit(f"{tool['name']} target {target} is missing from tools lock")


def load_tool_bundles(
    tools_root: Path,
    lock: dict,
    *,
    allow_partial_targets: bool = False,
    target: str | None = None,
) -> tuple[dict[str, dict], list[Path]]:
    grouped: dict[str, dict] = {}
    bin_roots: list[Path] = []
    locked_tools = lock_by_tool(lock)
    expected_targets = set(lock.get("targets", {}))
    if target is not None and target not in expected_targets:
        raise SystemExit(f"unknown target {target}; expected one of {', '.join(sorted(expected_targets))}")

    manifests = sorted(tools_root.rglob("tools.json"))
    if not manifests:
        raise SystemExit(f"no tools.json files found under {tools_root}")

    for manifest_path in manifests:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest_target = manifest["target"]
        if target is not None and manifest_target != target:
            continue

        bin_root = manifest_path.parent / "bin" / manifest_target
        if not bin_root.exists():
            raise SystemExit(f"tool binary directory not found: {bin_root}")
        bin_roots.append(manifest_path.parent / "bin")

        for tool in manifest["tools"]:
            name = tool["name"]
            if name not in locked_tools:
                raise SystemExit(f"tool bundle contains tool not present in lock: {name}")
            validate_tool_against_lock(tool, locked_tools[name], manifest_target)

            current = grouped.setdefault(
                name,
                {
                    "name": name,
                    "version": tool["version"],
                    "repository": tool["repository"],
                    "upstreamUrl": tool["upstreamUrl"],
                    "sourceTag": tool["sourceTag"],
                    "sourceCommit": tool["sourceCommit"],
                    "license": tool["license"],
                    "binaries": {},
                },
            )
            for key in ("version", "repository", "sourceTag", "sourceCommit", "license"):
                if current[key] != tool[key]:
                    raise SystemExit(f"inconsistent {key} for {name}: {current[key]} != {tool[key]}")
            current["binaries"][manifest_target] = {
                "targetTriple": tool["targetTriple"],
                "binaryPath": tool["binaryPath"],
                "sha256": tool["sha256"],
            }

    if target is not None and not grouped:
        raise SystemExit(f"no tools.json files found for target {target} under {tools_root}")

    for name in sorted(locked_tools):
        if name not in grouped:
            raise SystemExit(f"tool bundle missing locked tool: {name}")
        actual_targets = set(grouped[name]["binaries"])
        if allow_partial_targets:
            if not actual_targets:
                raise SystemExit(f"{name} bundle has no targets")
            unknown_targets = actual_targets - expected_targets
            if unknown_targets:
                raise SystemExit(f"{name} bundle contains unknown targets: {sorted(unknown_targets)}")
        elif actual_targets != expected_targets:
            raise SystemExit(
                f"{name} target matrix differs from lock: {sorted(actual_targets)} != {sorted(expected_targets)}"
            )

    return grouped, bin_roots


def write_manifest(plugin_dir: Path, grouped_tools: dict[str, dict], lock_file: Path) -> None:
    lock_path = lock_file.resolve()
    manifest = {
        "schemaVersion": 2,
        "builtAt": datetime.now(timezone.utc).replace(microsecond=0).isoformat(),
        "generatedBy": "scripts/ci/package-unica-plugin.py",
        "sourceLock": "third-party/tools.lock.json",
        "sourceLockSha256": sha256(lock_path),
        "tools": [grouped_tools[name] for name in sorted(grouped_tools)],
        "internalAdapters": [
            {
                "name": "v8std",
                "url": "https://ai.v8std.ru/mcp",
                "protocol": "streamable-http",
            }
        ],
    }
    path = plugin_dir / "third-party" / "manifest.json"
    path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def write_packaged_mcp_launcher(
    plugin_dir: Path, _grouped_tools: dict[str, dict] | None = None
) -> None:
    mcp_path = plugin_dir / ".mcp.json"
    mcp = json.loads(mcp_path.read_text(encoding="utf-8"))
    server = mcp["mcpServers"]["unica"]
    server["command"] = "git"
    server["args"] = [
        "-c",
        (
            'alias.unica-bootstrap=!f() { root="$PWD/${GIT_PREFIX:-}"; '
            'exec sh "${root}bootstrap/launch.sh" "$root"; }; f'
        ),
        "unica-bootstrap",
    ]
    server["cwd"] = "."
    server["note"] = (
        "Single public Unica stdio MCP orchestrator. The public Git package enters "
        "through a command-scoped Git shell alias, selects a native bootstrap for "
        "the host, verifies the pinned runtime, and transparently launches unica."
    )
    mcp_path.write_text(json.dumps(mcp, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def write_official_marketplace(source_path: Path, dest_path: Path, *, marketplace_name: str = PLUGIN_ID) -> None:
    data = json.loads(source_path.read_text(encoding="utf-8"))
    data["name"] = marketplace_name
    data.setdefault("interface", {})["displayName"] = DISPLAY_NAME

    if len(data.get("plugins", [])) != 1:
        raise SystemExit("Unica marketplace metadata must contain exactly one plugin")

    plugin = data["plugins"][0]
    plugin["name"] = PLUGIN_ID
    plugin["source"] = {
        "source": "local",
        "path": f"./plugins/{PLUGIN_ID}",
    }
    plugin["category"] = plugin.get("category", "Coding")

    dest_path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def assert_archive_clean(marketplace_dir: Path) -> None:
    for path in marketplace_dir.rglob("*"):
        rel = path.relative_to(marketplace_dir)
        parts = set(rel.parts)
        if parts & DISALLOWED_ARCHIVE_PARTS:
            raise SystemExit(f"archive contains disallowed path: {rel}")
        if path.name == ".DS_Store" or path.suffix in {".pyc", ".pyo"}:
            raise SystemExit(f"archive contains generated file: {rel}")
        if path.is_file() and path.name.endswith((".tar.gz", ".zip")):
            raise SystemExit(f"archive contains nested package artifact: {rel}")


def archive_base_name(version: str, *, target: str | None = None) -> str:
    if target:
        return f"unica-codex-marketplace-{target}"
    return f"unica-codex-marketplace-{version}"


def make_archives(marketplace_dir: Path, out_dir: Path, version: str, *, target: str | None = None) -> None:
    base_name = archive_base_name(version, target=target)
    tar_path = out_dir / f"{base_name}.tar.gz"
    zip_path = out_dir / f"{base_name}.zip"

    with tarfile.open(tar_path, "w:gz") as tf:
        tf.add(marketplace_dir, arcname=base_name)

    with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for path in sorted(marketplace_dir.rglob("*")):
            zf.write(path, Path(base_name) / path.relative_to(marketplace_dir))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument("--tools-root", type=Path, required=True)
    parser.add_argument("--lock-file", type=Path, default=Path("plugins/unica/third-party/tools.lock.json"))
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--marketplace-name", default=PLUGIN_ID)
    parser.add_argument("--allow-partial-targets", action="store_true")
    parser.add_argument("--no-archives", action="store_true")
    parser.add_argument("--target")
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    lock_file = (repo_root / args.lock_file).resolve() if not args.lock_file.is_absolute() else args.lock_file.resolve()
    plugin_src = repo_root / "plugins" / "unica"
    marketplace_src = repo_root / ".agents" / "plugins" / "marketplace.json"
    if not plugin_src.exists():
        raise SystemExit(f"plugin source not found: {plugin_src}")
    if not marketplace_src.exists():
        raise SystemExit(f"marketplace source not found: {marketplace_src}")

    plugin_json = json.loads((plugin_src / ".codex-plugin" / "plugin.json").read_text(encoding="utf-8"))
    version = plugin_json["version"]

    marketplace_dir = args.out_dir / "marketplace"
    shutil.rmtree(marketplace_dir, ignore_errors=True)
    marketplace_dir.mkdir(parents=True, exist_ok=True)
    plugin_dst = marketplace_dir / "plugins" / "unica"
    copy_tracked_plugin_source(repo_root, plugin_src, plugin_dst)

    marketplace_dst = marketplace_dir / ".agents" / "plugins"
    marketplace_dst.mkdir(parents=True, exist_ok=True)
    write_official_marketplace(
        marketplace_src,
        marketplace_dst / "marketplace.json",
        marketplace_name=args.marketplace_name,
    )

    lock = load_lock(lock_file)
    grouped_tools, bin_roots = load_tool_bundles(
        args.tools_root.resolve(),
        lock,
        allow_partial_targets=args.allow_partial_targets,
        target=args.target,
    )
    for bin_root in bin_roots:
        for target_dir in bin_root.iterdir():
            if target_dir.is_dir():
                if args.target is not None and target_dir.name != args.target:
                    continue
                copy_binary_tree(target_dir, plugin_dst / "bin" / target_dir.name)

    write_manifest(plugin_dst, grouped_tools, lock_file)
    write_packaged_mcp_launcher(plugin_dst, grouped_tools)

    json.loads((plugin_dst / ".codex-plugin" / "plugin.json").read_text(encoding="utf-8"))
    json.loads((plugin_dst / ".mcp.json").read_text(encoding="utf-8"))
    json.loads((plugin_dst / "third-party" / "manifest.json").read_text(encoding="utf-8"))
    json.loads((marketplace_dst / "marketplace.json").read_text(encoding="utf-8"))
    assert_archive_clean(marketplace_dir)

    args.out_dir.mkdir(parents=True, exist_ok=True)
    if not args.no_archives:
        make_archives(marketplace_dir, args.out_dir, version, target=args.target)


if __name__ == "__main__":
    main()
