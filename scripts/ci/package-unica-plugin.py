#!/usr/bin/env python3
"""Assemble a Codex marketplace package from built Unica tool artifacts."""

from __future__ import annotations

import argparse
import json
import shutil
import tarfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path


def copytree(src: Path, dst: Path, *, ignore: set[str] | None = None) -> None:
    ignore = ignore or set()
    if dst.exists():
        shutil.rmtree(dst)

    def _ignore(_dir: str, names: list[str]) -> set[str]:
        return set(names) & ignore

    shutil.copytree(src, dst, ignore=_ignore)


def load_tool_bundles(tools_root: Path) -> tuple[dict[str, dict], list[Path]]:
    grouped: dict[str, dict] = {}
    bin_roots: list[Path] = []

    manifests = sorted(tools_root.rglob("tools.json"))
    if not manifests:
        raise SystemExit(f"no tools.json files found under {tools_root}")

    for manifest_path in manifests:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        target = manifest["target"]
        bin_root = manifest_path.parent / "bin" / target
        if not bin_root.exists():
            raise SystemExit(f"tool binary directory not found: {bin_root}")
        bin_roots.append(manifest_path.parent / "bin")

        for tool in manifest["tools"]:
            name = tool["name"]
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
            current["binaries"][target] = {
                "targetTriple": tool["targetTriple"],
                "binaryPath": tool["binaryPath"],
                "sha256": tool["sha256"],
            }

    return grouped, bin_roots


def write_manifest(plugin_dir: Path, grouped_tools: dict[str, dict]) -> None:
    manifest = {
        "schemaVersion": 2,
        "builtAt": datetime.now(timezone.utc).replace(microsecond=0).isoformat(),
        "tools": [grouped_tools[name] for name in sorted(grouped_tools)],
        "remoteMcpServers": [
            {
                "name": "unica-v8std",
                "url": "https://ai.v8std.ru/mcp",
                "protocol": "streamable-http",
            }
        ],
    }
    path = plugin_dir / "third-party" / "manifest.json"
    path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def make_archives(marketplace_dir: Path, out_dir: Path, version: str) -> None:
    base_name = f"unica-codex-marketplace-{version}"
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
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    plugin_src = repo_root / "plugins" / "unica"
    marketplace_src = repo_root / ".agents" / "plugins" / "marketplace.json"
    if not plugin_src.exists():
        raise SystemExit(f"plugin source not found: {plugin_src}")
    if not marketplace_src.exists():
        raise SystemExit(f"marketplace source not found: {marketplace_src}")

    plugin_json = json.loads((plugin_src / ".codex-plugin" / "plugin.json").read_text(encoding="utf-8"))
    version = plugin_json["version"]

    marketplace_dir = args.out_dir / "marketplace"
    plugin_dst = marketplace_dir / "plugins" / "unica"
    copytree(plugin_src, plugin_dst, ignore={"bin"})

    marketplace_dst = marketplace_dir / ".agents" / "plugins"
    marketplace_dst.mkdir(parents=True, exist_ok=True)
    shutil.copy2(marketplace_src, marketplace_dst / "marketplace.json")

    grouped_tools, bin_roots = load_tool_bundles(args.tools_root.resolve())
    for bin_root in bin_roots:
        for target_dir in bin_root.iterdir():
            if target_dir.is_dir():
                copytree(target_dir, plugin_dst / "bin" / target_dir.name)

    write_manifest(plugin_dst, grouped_tools)

    json.loads((plugin_dst / ".codex-plugin" / "plugin.json").read_text(encoding="utf-8"))
    json.loads((plugin_dst / ".mcp.json").read_text(encoding="utf-8"))
    json.loads((plugin_dst / "third-party" / "manifest.json").read_text(encoding="utf-8"))

    args.out_dir.mkdir(parents=True, exist_ok=True)
    make_archives(marketplace_dir, args.out_dir, version)


if __name__ == "__main__":
    main()
