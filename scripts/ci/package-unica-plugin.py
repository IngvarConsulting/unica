#!/usr/bin/env python3
"""Assemble a Codex marketplace package from built Unica tool artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
from datetime import datetime, timezone
from pathlib import Path


PLUGIN_ID = "unica"
DISPLAY_NAME = "Unica"
SOURCE_PACKAGE_IGNORES = {"bin", ".DS_Store", "__pycache__", ".pytest_cache"}
DISALLOWED_ARCHIVE_PARTS = {".build", "dist", "__pycache__", ".pytest_cache"}
SUPPORTED_TARGETS = {
    "darwin-arm64": ("aarch64-apple-darwin", "unica-bootstrap"),
    "linux-x64": ("x86_64-unknown-linux-gnu", "unica-bootstrap"),
    "win-x64": ("x86_64-pc-windows-msvc", "unica-bootstrap.exe"),
}


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


def write_local_debug_mcp_launcher(plugin_dir: Path, target: str) -> None:
    if target not in SUPPORTED_TARGETS:
        raise SystemExit(f"unsupported local debug target: {target}")
    executable = "unica.exe" if target == "win-x64" else "unica"
    mcp_path = plugin_dir / ".mcp.json"
    mcp = json.loads(mcp_path.read_text(encoding="utf-8"))
    server = mcp["mcpServers"]["unica"]
    server["command"] = f"./bin/{target}/{executable}"
    server["args"] = []
    server["cwd"] = "."
    server["note"] = "Development-only current-host Unica MCP binary."
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


def write_public_marketplace(source_path: Path, dest_path: Path, *, release_tag: str) -> None:
    data = json.loads(source_path.read_text(encoding="utf-8"))
    data["name"] = PLUGIN_ID
    data.setdefault("interface", {})["displayName"] = DISPLAY_NAME
    if len(data.get("plugins", [])) != 1:
        raise SystemExit("Unica marketplace metadata must contain exactly one plugin")

    plugin = data["plugins"][0]
    plugin["name"] = PLUGIN_ID
    plugin["source"] = {
        "source": "git-subdir",
        "url": "https://github.com/IngvarConsulting/unica-marketplace.git",
        "path": "./plugins/unica",
        "ref": release_tag,
    }
    plugin.setdefault("policy", {})["installation"] = "AVAILABLE"
    plugin["category"] = plugin.get("category", "Coding")
    dest_path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def _lower_hex(value: str, length: int) -> bool:
    return len(value) == length and all(ch in "0123456789abcdef" for ch in value)


def load_runtime_metadata(metadata_root: Path, *, plugin_version: str) -> dict[str, dict]:
    manifests = sorted(metadata_root.rglob("unica-runtime-*.json"))
    if not manifests:
        raise SystemExit(f"no runtime metadata found under {metadata_root}")
    targets: dict[str, dict] = {}
    for path in manifests:
        data = json.loads(path.read_text(encoding="utf-8"))
        target = data.get("target")
        if target not in SUPPORTED_TARGETS:
            raise SystemExit(f"unsupported runtime metadata target: {target}")
        if target in targets:
            raise SystemExit(f"duplicate runtime metadata target: {target}")
        target_triple, executable = SUPPORTED_TARGETS[target]
        if data.get("schemaVersion") != 1:
            raise SystemExit(f"unsupported runtime metadata schema for {target}")
        if data.get("pluginVersion") != plugin_version:
            raise SystemExit(
                f"runtime metadata version for {target} differs from plugin: "
                f"{data.get('pluginVersion')} != {plugin_version}"
            )
        if data.get("targetTriple") != target_triple:
            raise SystemExit(f"runtime target triple mismatch for {target}")
        asset = data.get("asset", {})
        expected_asset = f"unica-runtime-{target}.tar.gz"
        if asset.get("name") != expected_asset or asset.get("mediaType") != "application/gzip":
            raise SystemExit(f"runtime asset identity mismatch for {target}")
        if not _lower_hex(asset.get("sha256", ""), 64):
            raise SystemExit(f"invalid runtime asset checksum for {target}")
        expected_entrypoint = f"bin/{target}/{'unica.exe' if executable.endswith('.exe') else 'unica'}"
        if data.get("entrypoint") != expected_entrypoint:
            raise SystemExit(f"runtime entrypoint mismatch for {target}")
        files = data.get("files")
        if not isinstance(files, list) or not files:
            raise SystemExit(f"runtime file list is empty for {target}")
        paths: set[str] = set()
        for runtime_file in files:
            relative = runtime_file.get("path", "")
            rel_path = Path(relative)
            if (
                not relative
                or "\\" in relative
                or rel_path.is_absolute()
                or ".." in rel_path.parts
                or relative in paths
            ):
                raise SystemExit(f"unsafe or duplicate runtime file for {target}: {relative}")
            paths.add(relative)
            if not _lower_hex(runtime_file.get("sha256", ""), 64):
                raise SystemExit(f"invalid runtime file checksum for {target}: {relative}")
        if expected_entrypoint not in paths:
            raise SystemExit(f"runtime entrypoint is not declared for {target}")
        targets[target] = data

    if set(targets) != set(SUPPORTED_TARGETS):
        raise SystemExit(
            f"runtime metadata targets {sorted(targets)} != {sorted(SUPPORTED_TARGETS)}"
        )
    return targets


def copy_bootstrap_matrix(bootstrap_root: Path, plugin_dir: Path) -> None:
    for target, (_triple, executable) in SUPPORTED_TARGETS.items():
        candidates = [
            path
            for path in bootstrap_root.rglob(executable)
            if path.parts[-4:] == ("bootstrap", "bin", target, executable)
        ]
        if len(candidates) != 1:
            raise SystemExit(
                f"expected exactly one bootstrap for {target} under {bootstrap_root}; "
                f"found {len(candidates)}"
            )
        source = candidates[0]
        if source.is_symlink() or not source.is_file():
            raise SystemExit(f"bootstrap must be a regular file: {source}")
        destination = plugin_dir / "bootstrap" / "bin" / target / executable
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        if not destination.name.endswith(".exe"):
            destination.chmod(destination.stat().st_mode | 0o755)


def write_release_runtime_manifest(
    plugin_dir: Path,
    metadata: dict[str, dict],
    *,
    plugin_version: str,
    release_tag: str,
    source_commit: str,
) -> None:
    expected_tag = f"v{plugin_version}"
    if release_tag != expected_tag:
        raise SystemExit(f"release tag {release_tag} != {expected_tag}")
    if not _lower_hex(source_commit, 40):
        raise SystemExit("source commit must be 40 lowercase hexadecimal characters")

    targets = {}
    for target in sorted(metadata):
        item = metadata[target]
        asset = dict(item["asset"])
        asset["url"] = (
            "https://github.com/IngvarConsulting/unica/releases/download/"
            f"{release_tag}/{asset['name']}"
        )
        targets[target] = {
            "asset": asset,
            "files": item["files"],
            "entrypoint": item["entrypoint"],
        }
    manifest = {
        "schemaVersion": 1,
        "pluginVersion": plugin_version,
        "development": False,
        "source": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "commit": source_commit,
        },
        "release": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "tag": release_tag,
        },
        "targets": targets,
    }
    (plugin_dir / "runtime-manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


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


def package_local_debug(
    *,
    repo_root: Path,
    tools_root: Path,
    lock_file: Path,
    out_dir: Path,
    marketplace_name: str,
    target: str,
) -> None:
    plugin_src = repo_root / "plugins" / "unica"
    marketplace_src = repo_root / ".agents" / "plugins" / "marketplace.json"
    marketplace_dir = out_dir / "marketplace"
    shutil.rmtree(marketplace_dir, ignore_errors=True)
    plugin_dst = marketplace_dir / "plugins" / "unica"
    copy_tracked_plugin_source(repo_root, plugin_src, plugin_dst)

    marketplace_dst = marketplace_dir / ".agents" / "plugins"
    marketplace_dst.mkdir(parents=True, exist_ok=True)
    write_official_marketplace(
        marketplace_src,
        marketplace_dst / "marketplace.json",
        marketplace_name=marketplace_name,
    )

    lock = load_lock(lock_file)
    grouped_tools, bin_roots = load_tool_bundles(
        tools_root,
        lock,
        allow_partial_targets=True,
        target=target,
    )
    for bin_root in bin_roots:
        source = bin_root / target
        if source.is_dir():
            copy_binary_tree(source, plugin_dst / "bin" / target)
    write_manifest(plugin_dst, grouped_tools, lock_file)
    write_local_debug_mcp_launcher(plugin_dst, target)
    assert_archive_clean(marketplace_dir)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument("--runtime-metadata-root", type=Path)
    parser.add_argument("--bootstrap-root", type=Path)
    parser.add_argument("--release-tag")
    parser.add_argument("--source-commit")
    parser.add_argument("--local-debug-target")
    parser.add_argument("--tools-root", type=Path)
    parser.add_argument("--lock-file", type=Path, default=Path("plugins/unica/third-party/tools.lock.json"))
    parser.add_argument("--marketplace-name", default="unica-dev")
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    if args.local_debug_target:
        if args.tools_root is None:
            raise SystemExit("--tools-root is required with --local-debug-target")
        lock_file = args.lock_file if args.lock_file.is_absolute() else repo_root / args.lock_file
        package_local_debug(
            repo_root=repo_root,
            tools_root=args.tools_root.resolve(),
            lock_file=lock_file.resolve(),
            out_dir=args.out_dir.resolve(),
            marketplace_name=args.marketplace_name,
            target=args.local_debug_target,
        )
        return

    missing = [
        name
        for name, value in (
            ("--runtime-metadata-root", args.runtime_metadata_root),
            ("--bootstrap-root", args.bootstrap_root),
            ("--release-tag", args.release_tag),
            ("--source-commit", args.source_commit),
        )
        if value is None
    ]
    if missing:
        raise SystemExit(f"release packaging requires: {', '.join(missing)}")
    plugin_src = repo_root / "plugins" / "unica"
    marketplace_src = repo_root / ".agents" / "plugins" / "marketplace.json"
    if not plugin_src.exists():
        raise SystemExit(f"plugin source not found: {plugin_src}")
    if not marketplace_src.exists():
        raise SystemExit(f"marketplace source not found: {marketplace_src}")

    plugin_json = json.loads((plugin_src / ".codex-plugin" / "plugin.json").read_text(encoding="utf-8"))
    version = plugin_json["version"]
    metadata = load_runtime_metadata(args.runtime_metadata_root.resolve(), plugin_version=version)

    marketplace_dir = args.out_dir / "marketplace"
    shutil.rmtree(marketplace_dir, ignore_errors=True)
    marketplace_dir.mkdir(parents=True, exist_ok=True)
    plugin_dst = marketplace_dir / "plugins" / "unica"
    copy_tracked_plugin_source(repo_root, plugin_src, plugin_dst)

    marketplace_dst = marketplace_dir / ".agents" / "plugins"
    marketplace_dst.mkdir(parents=True, exist_ok=True)
    write_public_marketplace(
        marketplace_src,
        marketplace_dst / "marketplace.json",
        release_tag=args.release_tag,
    )

    copy_bootstrap_matrix(args.bootstrap_root.resolve(), plugin_dst)
    write_release_runtime_manifest(
        plugin_dst,
        metadata,
        plugin_version=version,
        release_tag=args.release_tag,
        source_commit=args.source_commit,
    )
    write_packaged_mcp_launcher(plugin_dst)

    json.loads((plugin_dst / ".codex-plugin" / "plugin.json").read_text(encoding="utf-8"))
    json.loads((plugin_dst / ".mcp.json").read_text(encoding="utf-8"))
    json.loads((plugin_dst / "runtime-manifest.json").read_text(encoding="utf-8"))
    json.loads((marketplace_dst / "marketplace.json").read_text(encoding="utf-8"))
    assert_archive_clean(marketplace_dir)

    args.out_dir.mkdir(parents=True, exist_ok=True)


if __name__ == "__main__":
    main()
