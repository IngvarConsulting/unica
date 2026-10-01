#!/usr/bin/env python3
"""Verify re-downloaded Unica runtime release assets byte-for-byte."""

from __future__ import annotations

import argparse
import hashlib
import json
import tarfile
from pathlib import Path


TARGETS = {"darwin-arm64", "linux-x64", "win-x64"}
SCHEMA_VERSION = 1
VERIFICATION_SOURCES = {"local-build", "re-downloaded"}
# Четыре шага верификации, каждый ставит свой флаг сам, когда прошёл. Отчёт
# только переписывает их: флаг, который никто не поставил, остаётся `False`.
CHECKS = ("artifactSet", "archiveChecksum", "memberChecksums", "memberMetadata")


def new_checks() -> dict[str, bool]:
    return {name: False for name in CHECKS}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


# Имя артефакта ядра. Остальные архивы на цели — движки.
CORE_ARTIFACT = "unica"


def published_artifacts(
    asset_dir: Path, target: str, checks: dict[str, bool] | None = None
) -> "list[str]":
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
    if checks is not None:
        checks["artifactSet"] = True
    return found


def verify_runtime_asset_pair(
    asset_dir: Path,
    target: str,
    artifact: str = CORE_ARTIFACT,
    checks: dict[str, bool] | None = None,
) -> str:
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
    if checks is not None:
        checks["archiveChecksum"] = True

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
        if checks is not None:
            checks["memberChecksums"] = True
        for member in members:
            expected_mode = 0o755 if expected[member.name].get("executable") else 0o644
            if member.mode != expected_mode or member.mtime != 0:
                raise SystemExit(f"published runtime member metadata mismatch: {member.name}")
        if checks is not None:
            checks["memberMetadata"] = True

    version = metadata.get("pluginVersion", "")
    if not version:
        raise SystemExit("published runtime target/version matrix is inconsistent")
    return version


def verify_release_target(
    asset_dir: Path, target: str, checks: dict[str, bool] | None = None
) -> str:
    """Одна цель целиком: и состав выпуска, и байты каждой пары.

    Состав проверяется здесь, потому что здесь он и создаётся: лишний архив,
    замеченный после выкладки, — уже опубликованный лишний архив.
    """
    versions = {
        verify_runtime_asset_pair(asset_dir, target, artifact, checks)
        for artifact in published_artifacts(asset_dir, target, checks)
    }
    if len(versions) != 1:
        raise SystemExit(f"published runtime versions disagree for {target}: {sorted(versions)}")
    return versions.pop()


def verify_release_assets(asset_dir: Path, checks: dict[str, bool] | None = None) -> str:
    # Флаг матрицы стоит только когда шаг прошёл на каждой цели.
    per_target: list[dict[str, bool]] = []
    versions = set()
    for target in sorted(TARGETS):
        target_checks = new_checks()
        versions.add(verify_release_target(asset_dir, target, target_checks))
        per_target.append(target_checks)
    if len(versions) != 1:
        raise SystemExit("published runtime target/version matrix is inconsistent")
    if checks is not None:
        for name in CHECKS:
            checks[name] = all(item[name] for item in per_target)
    return versions.pop()


def build_verification_report(
    asset_dir: Path, target: str | None = None, *, source: str
) -> dict:
    """Return a durable outcome whose `checks` are the verifier's own flags.

    Каждый ключ ставит тот шаг, который его проверил (#701); отчёт ничего не
    дописывает. Проваленный шаг поднимает `SystemExit` раньше, чем отчёт
    будет собран, так что на диске `false` не появляется — но и `true` здесь
    только наблюдённый.
    """
    if source not in VERIFICATION_SOURCES:
        raise SystemExit(f"unsupported runtime asset verification source: {source}")
    targets = [target] if target else sorted(TARGETS)
    checks = new_checks()
    if target:
        if target not in TARGETS:
            raise SystemExit(f"unsupported runtime asset target: {target}")
        version = verify_release_target(asset_dir, target, checks)
    else:
        version = verify_release_assets(asset_dir, checks)
    return {
        "schemaVersion": SCHEMA_VERSION,
        "status": "passed" if all(checks.values()) else "failed",
        "source": source,
        "pluginVersion": version,
        "targets": targets,
        "checks": checks,
    }


def write_verification_report(report: dict, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--asset-dir", type=Path, required=True)
    parser.add_argument("--target", choices=sorted(TARGETS))
    parser.add_argument("--report", type=Path)
    parser.add_argument(
        "--source", choices=sorted(VERIFICATION_SOURCES), default="re-downloaded"
    )
    args = parser.parse_args()
    report = build_verification_report(
        args.asset_dir,
        args.target,
        source=args.source,
    )
    if args.report:
        write_verification_report(report, args.report)
    print(f"verified Unica runtime assets: {report['pluginVersion']}")


if __name__ == "__main__":
    main()
