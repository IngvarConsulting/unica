#!/usr/bin/env python3
"""Create one deterministic, self-contained Unica runtime archive."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import tarfile
from pathlib import Path, PurePosixPath


SUPPORTED_TARGETS = {
    "darwin-arm64": "aarch64-apple-darwin",
    "linux-x64": "x86_64-unknown-linux-gnu",
    "win-x64": "x86_64-pc-windows-msvc",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_relative_path(value: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if not value or "\\" in value or path.is_absolute() or ".." in path.parts:
        raise SystemExit(f"unsafe runtime path: {value}")
    return path


def load_bundle(bundle_root: Path) -> tuple[dict, list[tuple[PurePosixPath, Path, bool]]]:
    manifest_path = bundle_root / "tools.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    target = manifest.get("target")
    if target not in SUPPORTED_TARGETS:
        raise SystemExit(f"unsupported runtime target: {target}")
    if manifest.get("targetTriple") != SUPPORTED_TARGETS[target]:
        raise SystemExit(f"runtime target triple mismatch for {target}")

    tools = manifest.get("tools")
    if not isinstance(tools, list) or not tools:
        raise SystemExit(f"runtime tool manifest is empty: {manifest_path}")

    seen_names: set[str] = set()
    seen_paths: set[str] = set()
    runtime_files: list[tuple[PurePosixPath, Path, bool]] = []
    for tool in tools:
        name = tool.get("name")
        if not name or name in seen_names:
            raise SystemExit(f"duplicate or missing runtime tool name: {name}")
        seen_names.add(name)
        if tool.get("target") not in (None, target):
            raise SystemExit(f"runtime tool {name} target differs from {target}")
        if tool.get("targetTriple", SUPPORTED_TARGETS[target]) != SUPPORTED_TARGETS[target]:
            raise SystemExit(f"runtime tool {name} target triple differs from {target}")

        relative = safe_relative_path(tool.get("binaryPath", ""))
        expected_prefix = PurePosixPath("bin") / target
        if relative.parent != expected_prefix:
            raise SystemExit(f"runtime tool {name} is outside {expected_prefix}: {relative}")
        if relative.as_posix() in seen_paths:
            raise SystemExit(f"duplicate runtime binary path: {relative}")
        seen_paths.add(relative.as_posix())

        binary = bundle_root.joinpath(*relative.parts)
        if binary.is_symlink():
            raise SystemExit(f"runtime binary must not be a symlink: {relative}")
        if not binary.is_file():
            raise SystemExit(f"runtime binary is missing: {relative}")
        if sha256(binary) != tool.get("sha256"):
            raise SystemExit(f"runtime binary checksum mismatch: {relative}")
        runtime_files.append((relative, binary, True))

    unica = [tool for tool in tools if tool["name"] == "unica"]
    if len(unica) != 1:
        raise SystemExit("runtime bundle must contain exactly one unica tool")
    plugin_version = unica[0].get("version")
    if not plugin_version:
        raise SystemExit("unica runtime version is missing")

    generated_manifest = {
        "schemaVersion": 2,
        "generatedBy": "scripts/ci/package-unica-runtime.py",
        "targetTriple": SUPPORTED_TARGETS[target],
        "tools": [
            {
                key: tool[key]
                for key in (
                    "name",
                    "version",
                    "repository",
                    "upstreamUrl",
                    "sourceTag",
                    "sourceCommit",
                    "license",
                    "binaryPath",
                    "sha256",
                )
                if key in tool
            }
            for tool in sorted(tools, key=lambda item: item["name"])
        ],
        "internalAdapters": [
            {
                "name": "v8std",
                "url": "https://ai.v8std.ru/mcp",
                "protocol": "streamable-http",
            }
        ],
    }
    manifest_bytes = (
        json.dumps(generated_manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")
    return {
        "target": target,
        "targetTriple": SUPPORTED_TARGETS[target],
        "pluginVersion": plugin_version,
        "entrypoint": unica[0]["binaryPath"],
        "toolManifestBytes": manifest_bytes,
    }, runtime_files


def add_tar_member(archive: tarfile.TarFile, name: str, payload: bytes, executable: bool) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    info.mode = 0o755 if executable else 0o644
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    info.mtime = 0
    archive.addfile(info, io.BytesIO(payload))


def package_runtime(bundle_root: Path, out_dir: Path) -> tuple[Path, Path]:
    bundle_root = bundle_root.resolve()
    bundle, source_files = load_bundle(bundle_root)
    target = bundle["target"]
    out_dir.mkdir(parents=True, exist_ok=True)
    archive_path = out_dir / f"unica-runtime-{target}.tar.gz"
    metadata_path = out_dir / f"unica-runtime-{target}.json"

    payloads = [
        (relative.as_posix(), path.read_bytes(), executable)
        for relative, path, executable in source_files
    ]
    payloads.append(("third-party/manifest.json", bundle["toolManifestBytes"], False))
    payloads.sort(key=lambda item: item[0])

    with archive_path.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for name, payload, executable in payloads:
                    add_tar_member(archive, name, payload, executable)

    files = [
        {
            "path": name,
            "sha256": hashlib.sha256(payload).hexdigest(),
            "executable": executable,
        }
        for name, payload, executable in payloads
    ]
    metadata = {
        "schemaVersion": 1,
        "target": target,
        "targetTriple": bundle["targetTriple"],
        "pluginVersion": bundle["pluginVersion"],
        "asset": {
            "name": archive_path.name,
            "mediaType": "application/gzip",
            "sha256": sha256(archive_path),
        },
        "files": files,
        "entrypoint": bundle["entrypoint"],
    }
    metadata_path.write_text(
        json.dumps(metadata, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return archive_path, metadata_path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bundle-root", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    package_runtime(args.bundle_root, args.out_dir)


if __name__ == "__main__":
    main()
