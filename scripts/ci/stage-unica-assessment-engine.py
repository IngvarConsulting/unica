#!/usr/bin/env python3
"""Stage the verified engine closures required by one release assessment."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import tempfile
from pathlib import Path, PurePosixPath


SHA256 = re.compile(r"^[0-9a-f]{64}$")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_relative_path(value: object) -> PurePosixPath:
    if not isinstance(value, str):
        raise SystemExit(f"unsafe runtime path: {value}")
    path = PurePosixPath(value)
    if (
        not value
        or "\\" in value
        or "\x00" in value
        or path.is_absolute()
        or any(part in ("", ".", "..") for part in value.split("/"))
        or path.as_posix() != value
    ):
        raise SystemExit(f"unsafe runtime path: {value}")
    return path


def selected_runtime_files(
    bundle_root: Path,
    manifest: dict,
    *,
    artifacts: set[str],
) -> list[tuple[PurePosixPath, Path, bool]]:
    if manifest.get("schemaVersion") != 2:
        raise SystemExit(
            f"unsupported tools manifest schemaVersion: {manifest.get('schemaVersion')}"
        )
    target = manifest.get("target")
    if not isinstance(target, str) or not target:
        raise SystemExit("tools manifest target must be a non-empty string")
    expected_prefix = PurePosixPath("bin") / target
    declared = manifest.get("runtimeFiles")
    if not isinstance(declared, list) or not declared:
        raise SystemExit("runtimeFiles closure must be a non-empty array")

    found_artifacts: set[str] = set()
    found_source_artifacts: set[str] = set()
    by_path: dict[str, tuple[PurePosixPath, Path, bool]] = {}
    for index, item in enumerate(declared):
        if not isinstance(item, dict):
            raise SystemExit(f"runtimeFiles[{index}] must be an object")
        artifact = item.get("artifact")
        if artifact not in artifacts:
            continue
        found_artifacts.add(artifact)
        if "path" not in item:
            # A foreign archive envelope exists only in delivered layout. The
            # assessment overlays the verified plugin layout built from it.
            continue

        relative = safe_relative_path(item["path"])
        if relative.parent != expected_prefix and expected_prefix not in relative.parents:
            raise SystemExit(f"runtime file is outside {expected_prefix}: {relative}")
        relative_text = relative.as_posix()
        if relative_text in by_path:
            raise SystemExit(f"duplicate selected runtime file path: {relative}")

        digest = item.get("sha256")
        if not isinstance(digest, str) or not SHA256.fullmatch(digest):
            raise SystemExit(f"invalid runtime file sha256: {relative}")
        size = item.get("size")
        if not isinstance(size, int) or isinstance(size, bool) or size < 0:
            raise SystemExit(f"invalid runtime file size: {relative}")
        executable = item.get("executable")
        if not isinstance(executable, bool):
            raise SystemExit(f"invalid runtime file executable flag: {relative}")

        source = bundle_root.joinpath(*relative.parts)
        if source.is_symlink():
            raise SystemExit(f"runtime file must not be a symlink: {relative}")
        if not source.is_file():
            raise SystemExit(f"runtime file is missing: {relative}")
        source_stat = source.stat()
        if source_stat.st_nlink != 1:
            raise SystemExit(f"runtime file must not be hard-linked: {relative}")
        if source_stat.st_size != size:
            raise SystemExit(f"runtime file size mismatch: {relative}")
        if sha256(source) != digest:
            raise SystemExit(f"runtime file checksum mismatch: {relative}")
        if os.name != "nt":
            actual_executable = bool(stat.S_IMODE(source_stat.st_mode) & 0o111)
            if actual_executable != executable:
                raise SystemExit(f"runtime file executable mode mismatch: {relative}")

        by_path[relative_text] = (relative, source, executable)
        found_source_artifacts.add(artifact)

    missing = artifacts - found_artifacts
    if missing:
        raise SystemExit(f"assessment artifacts are not declared: {sorted(missing)}")
    without_sources = artifacts - found_source_artifacts
    if without_sources:
        raise SystemExit(
            f"assessment artifacts have no plugin-layout files: {sorted(without_sources)}"
        )
    return [by_path[path] for path in sorted(by_path)]


def stage_engine(bundle_root: Path, out_dir: Path, *, artifacts: set[str]) -> None:
    if not artifacts:
        raise SystemExit("at least one --artifact is required")
    for artifact in artifacts:
        if not artifact or "/" in artifact or "\\" in artifact:
            raise SystemExit(f"unsafe artifact name: {artifact}")

    bundle_root = bundle_root.resolve()
    manifest_path = bundle_root / "tools.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    files = selected_runtime_files(bundle_root, manifest, artifacts=artifacts)

    out_dir = out_dir.resolve()
    if out_dir.exists():
        raise SystemExit(f"assessment engine output already exists: {out_dir}")
    out_dir.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(
        tempfile.mkdtemp(prefix=f".{out_dir.name}.staging-", dir=out_dir.parent)
    )
    try:
        for relative, source, executable in files:
            destination = staging.joinpath(*relative.parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
            destination.chmod(0o755 if executable else 0o644)
        os.replace(staging, out_dir)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise

    print(
        f"staged {len(files)} files for {', '.join(sorted(artifacts))} into {out_dir}"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle-root", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--artifact", action="append", required=True)
    args = parser.parse_args()
    stage_engine(args.bundle_root, args.out_dir, artifacts=set(args.artifact))


if __name__ == "__main__":
    main()
