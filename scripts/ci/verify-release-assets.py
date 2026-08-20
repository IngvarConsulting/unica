#!/usr/bin/env python3
"""Verify re-downloaded Unica runtime release assets byte-for-byte."""

from __future__ import annotations

import argparse
import hashlib
import json
import tarfile
from pathlib import Path


TARGETS = {"darwin-arm64", "linux-x64", "win-x64"}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


# Имя артефакта ядра. Остальные архивы на цели — движки.
CORE_ARTIFACT = "unica"


def published_artifacts(asset_dir: Path, target: str) -> "list[str]":
    """Какие артефакты выпуск публикует архивом.

    Ровно один — ядро. Движки издаёт тулчейн под своими тегами, и лишний архив
    здесь означает, что выпуск снова начал перепубликовывать чужие байты.
    """
    suffix = f"-runtime-{target}.tar.gz"
    found = sorted(
        path.name[: -len(suffix)]
        for path in asset_dir.glob(f"*{suffix}")
        if path.name.endswith(suffix)
    )
    if found != [CORE_ARTIFACT]:
        raise SystemExit(
            f"published runtime archives for {target} must be [{CORE_ARTIFACT}], got {found}"
        )
    return found


def verify_runtime_asset_pair(asset_dir: Path, target: str, artifact: str = CORE_ARTIFACT) -> str:
    metadata_path = asset_dir / f"{artifact}-runtime-{target}.json"
    archive_path = asset_dir / f"{artifact}-runtime-{target}.tar.gz"
    if not metadata_path.is_file() or not archive_path.is_file():
        raise SystemExit(f"published runtime asset pair is missing for {target}")
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    if (
        metadata.get("schemaVersion") != 2
        or metadata.get("target") != target
        or metadata.get("artifact") != artifact
    ):
        raise SystemExit(f"published runtime metadata identity mismatch for {target}")
    if not metadata.get("version"):
        raise SystemExit(f"published runtime metadata has no artifact version for {target}")
    if metadata.get("asset", {}).get("name") != archive_path.name:
        raise SystemExit(f"published runtime archive name mismatch for {target}")
    if metadata["asset"].get("sha256") != sha256(archive_path):
        raise SystemExit(f"published runtime archive checksum mismatch for {target}")

    expected = {item["path"]: item for item in metadata.get("files", [])}
    with tarfile.open(archive_path, "r:gz") as archive:
        members = [member for member in archive.getmembers() if member.isfile()]
        actual_names = [member.name for member in members]
        if actual_names != sorted(expected):
            raise SystemExit(f"published runtime file set mismatch for {target}")
        for member in members:
            source = archive.extractfile(member)
            if source is None:
                raise SystemExit(f"cannot read published runtime member {member.name}")
            digest = hashlib.sha256(source.read()).hexdigest()
            if digest != expected[member.name]["sha256"]:
                raise SystemExit(f"published runtime member checksum mismatch: {member.name}")
            expected_mode = 0o755 if expected[member.name].get("executable") else 0o644
            if member.mode != expected_mode or member.mtime != 0:
                raise SystemExit(f"published runtime member metadata mismatch: {member.name}")

    version = metadata.get("pluginVersion", "")
    if not version:
        raise SystemExit("published runtime target/version matrix is inconsistent")
    return version


def verify_release_assets(asset_dir: Path) -> str:
    versions = set()
    for target in sorted(TARGETS):
        for artifact in published_artifacts(asset_dir, target):
            versions.add(verify_runtime_asset_pair(asset_dir, target, artifact))
    if len(versions) != 1:
        raise SystemExit("published runtime target/version matrix is inconsistent")
    return versions.pop()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--asset-dir", type=Path, required=True)
    parser.add_argument("--target", choices=sorted(TARGETS))
    args = parser.parse_args()
    version = (
        verify_runtime_asset_pair(args.asset_dir, args.target)
        if args.target
        else verify_release_assets(args.asset_dir)
    )
    print(f"verified published Unica runtime assets: {version}")


if __name__ == "__main__":
    main()
