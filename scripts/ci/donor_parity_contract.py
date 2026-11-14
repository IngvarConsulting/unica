#!/usr/bin/env python3
"""Offline integrity contract for the accepted cc-1c-skills donor snapshot.

The snapshot is the archive of the accepted donor commit kept for provenance.
Unica behaviour is not compared with it case by case; this module only checks
that the snapshot bytes, the baseline manifest and the provenance record agree.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
from pathlib import Path, PurePosixPath
from typing import Any, Iterable


HEX_40 = re.compile(r"^[0-9a-f]{40}$")
HEX_64 = re.compile(r"^[0-9a-f]{64}$")
CASE_EXECUTION_PROFILE = {
    "schemaVersion": 1,
    "platformVersion": "8.3.27",
    "exportFormat": "2.20",
    "emptyConfigProjection": {"from": "2.17", "to": "2.20"},
}
CASE_EXECUTION_PATH_PROJECTIONS = {
    "cfe-borrow": {"ext": "extension"},
}


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object: {path}")
    return value


def safe_relative_path(raw: str) -> Path:
    if not isinstance(raw, str) or not raw or "\\" in raw:
        raise ValueError(f"unsafe repository-relative path: {raw!r}")
    pure = PurePosixPath(raw)
    if (
        pure.is_absolute()
        or raw == "."
        or ".." in pure.parts
        or pure.as_posix() != raw
    ):
        raise ValueError(f"unsafe repository-relative path: {raw}")
    return Path(*pure.parts)


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sha256_json(value: object) -> str:
    payload = json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def case_execution_profile(case_id: str) -> dict[str, Any]:
    case_parts = safe_relative_path(case_id).parts
    if len(case_parts) != 2:
        raise ValueError(f"case id must be '<scope>/<case>': {case_id}")
    profile = dict(CASE_EXECUTION_PROFILE)
    projection = CASE_EXECUTION_PATH_PROJECTIONS.get(case_parts[0])
    if projection is not None:
        profile["workspacePathProjection"] = dict(projection)
    return profile


def discover_case_ids(snapshot_root: Path) -> list[str]:
    cases_root = snapshot_root / "cases"
    if not cases_root.is_dir():
        return []
    result = []
    for case_path in sorted(cases_root.glob("*/*.json")):
        if case_path.name.startswith("_"):
            continue
        result.append(
            f"{case_path.parent.name}/{case_path.stem}"
        )
    return result


def case_content_digest(snapshot_root: Path, case_id: str) -> str:
    case_parts = safe_relative_path(case_id).parts
    if len(case_parts) != 2:
        raise ValueError(f"case id must be '<scope>/<case>': {case_id}")
    scope, case_name = case_parts
    case_root = snapshot_root / "cases" / scope
    config_path = case_root / "_skill.json"
    case_path = case_root / f"{case_name}.json"
    config = load_json(config_path)
    case = load_json(case_path)

    dependencies = {config_path, case_path}
    _add_script_dependencies(snapshot_root, config.get("script"), dependencies)
    post_validate = config.get("postValidate")
    if isinstance(post_validate, dict):
        _add_script_dependencies(
            snapshot_root, post_validate.get("script"), dependencies
        )

    setup = case.get("setup") or config.get("setup") or "none"
    if setup == "empty-config":
        _add_script_dependencies(
            snapshot_root, "cf-init/scripts/cf-init", dependencies
        )
    elif isinstance(setup, str) and setup.startswith("fixture:"):
        fixture_name = setup.removeprefix("fixture:")
        fixture_relative = safe_relative_path(fixture_name)
        fixture_root = case_root / "fixtures" / fixture_relative
        if not fixture_root.is_dir():
            raise FileNotFoundError(f"case fixture does not exist: {fixture_root}")
        dependencies.update(_regular_files(fixture_root))
    elif setup not in ("none", None):
        raise ValueError(f"unsupported case setup: {setup!r}")

    for step in case.get("preRun") or []:
        if not isinstance(step, dict):
            raise ValueError(f"invalid preRun step in {case_id}")
        if "script" in step:
            _add_script_dependencies(
                snapshot_root, step.get("script"), dependencies
            )

    records = []
    for path in sorted(dependencies):
        _require_regular_file(path)
        records.append(
            {
                "path": path.relative_to(snapshot_root).as_posix(),
                "sha256": sha256_file(path),
            }
        )
    return sha256_json(
        {
            "executionProfile": case_execution_profile(case_id),
            "files": records,
        }
    )


def scope_content_digest(
    files: Iterable[dict[str, Any]], scope: str
) -> str:
    records = [
        {
            "localPath": item.get("localPath"),
            "upstreamPath": item.get("upstreamPath"),
            "sha256": item.get("sha256"),
        }
        for item in files
        if item.get("scope") == scope
    ]
    records.sort(key=lambda item: str(item["localPath"]))
    return sha256_json(records)


def validate_baseline(
    snapshot_root: Path,
    manifest: dict[str, Any],
    provenance: dict[str, Any],
) -> list[str]:
    errors: list[str] = []
    if manifest.get("schemaVersion") != 1:
        errors.append("donor baseline schemaVersion must be 1")

    upstream_id = manifest.get("upstreamId")
    if not isinstance(upstream_id, str) or not upstream_id:
        errors.append("donor baseline upstreamId is required")
    upstream = _find_upstream(provenance, upstream_id)
    if upstream is None:
        errors.append(f"provenance upstream is missing: {upstream_id!r}")
    else:
        for field in ("repository", "trackingRef"):
            if manifest.get(field) != upstream.get(field):
                errors.append(
                    f"{field} mismatch between donor baseline and provenance"
                )

    symlinks = _snapshot_symlinks(snapshot_root)
    for path in symlinks:
        errors.append(
            f"snapshot symlink is forbidden: "
            f"{path.relative_to(snapshot_root).as_posix()}"
        )

    actual_files = {
        path.relative_to(snapshot_root).as_posix()
        for path in _regular_files(snapshot_root)
    }
    scopes = manifest.get("scopes")
    if not isinstance(scopes, dict):
        errors.append("donor baseline scopes must be an object")
        scopes = {}
    files = manifest.get("files")
    if not isinstance(files, list):
        errors.append("donor baseline files must be an array")
        files = []

    manifest_paths: set[str] = set()
    upstream_paths: set[str] = set()
    for index, item in enumerate(files):
        label = f"files[{index}]"
        if not isinstance(item, dict):
            errors.append(f"{label} must be an object")
            continue
        local_raw = item.get("localPath")
        upstream_raw = item.get("upstreamPath")
        local_path = _validated_path(local_raw, label, "localPath", errors)
        _validated_path(upstream_raw, label, "upstreamPath", errors)
        if isinstance(local_raw, str):
            if local_raw in manifest_paths:
                errors.append(f"duplicate localPath: {local_raw}")
            manifest_paths.add(local_raw)
        if isinstance(upstream_raw, str):
            if upstream_raw in upstream_paths:
                errors.append(f"duplicate upstreamPath: {upstream_raw}")
            upstream_paths.add(upstream_raw)

        scope = item.get("scope")
        scope_data = scopes.get(scope) if isinstance(scope, str) else None
        if not isinstance(scope_data, dict):
            errors.append(f"{label} references unknown scope: {scope!r}")
        else:
            accepted = scope_data.get("acceptedCommit")
            if item.get("acceptedCommit") != accepted:
                errors.append(f"{label} acceptedCommit differs from its scope")

        if local_path is None:
            continue
        path = snapshot_root / local_path
        if path.is_symlink():
            errors.append(f"{label} points to a symlink: {local_raw}")
        elif not path.is_file():
            errors.append(f"{label} localPath does not exist: {local_raw}")
        elif not HEX_64.fullmatch(str(item.get("sha256") or "")):
            errors.append(f"{label} sha256 must be a lowercase 64-hex digest")
        elif sha256_file(path) != item.get("sha256"):
            errors.append(f"{label} sha256 mismatch: {local_raw}")

    for local_path in sorted(actual_files - manifest_paths):
        errors.append(f"unmanifested snapshot file: {local_path}")
    for local_path in sorted(manifest_paths - actual_files):
        errors.append(f"manifested snapshot file is missing: {local_path}")

    entries = {}
    if upstream is not None:
        entries = {
            entry.get("skill"): entry
            for entry in upstream.get("entries") or []
            if isinstance(entry, dict) and isinstance(entry.get("skill"), str)
        }
    for scope, scope_data in sorted(scopes.items()):
        if not isinstance(scope_data, dict):
            errors.append(f"scope {scope!r} must be an object")
            continue
        commit = scope_data.get("acceptedCommit")
        if not isinstance(commit, str) or not HEX_40.fullmatch(commit):
            errors.append(
                f"scope {scope!r} acceptedCommit must be a concrete 40-hex commit"
            )
        owner = scope_data.get("ownerSkill")
        entry = entries.get(owner)
        if upstream is not None and not isinstance(entry, dict):
            errors.append(
                f"scope {scope!r} ownerSkill is missing from provenance: {owner!r}"
            )
        elif isinstance(entry, dict):
            provenance_commit = entry.get("parityBaselineCommit")
            if provenance_commit != commit:
                errors.append(
                    f"scope {scope!r} provenance commit mismatch: "
                    f"{provenance_commit!r} != {commit!r}"
                )
        expected_scope_digest = scope_content_digest(files, scope)
        if scope_data.get("contentDigest") != expected_scope_digest:
            errors.append(f"scope {scope!r} contentDigest mismatch")

    cases = manifest.get("cases")
    if not isinstance(cases, dict):
        errors.append("donor baseline cases must be an object")
        cases = {}
    discovered_cases = set(discover_case_ids(snapshot_root))
    manifest_cases = set(cases)
    for case_id in sorted(discovered_cases - manifest_cases):
        errors.append(f"missing baseline case: {case_id}")
    for case_id in sorted(manifest_cases - discovered_cases):
        errors.append(f"baseline contains unknown case: {case_id}")
    for case_id in sorted(discovered_cases & manifest_cases):
        case_data = cases.get(case_id)
        if not isinstance(case_data, dict):
            errors.append(f"baseline case {case_id} must be an object")
            continue
        try:
            digest = case_content_digest(snapshot_root, case_id)
        except (FileNotFoundError, ValueError) as error:
            errors.append(f"cannot digest baseline case {case_id}: {error}")
            continue
        if case_data.get("contentDigest") != digest:
            errors.append(f"baseline case contentDigest mismatch: {case_id}")

    return errors


def validate_repository_contract(repo_root: Path) -> list[str]:
    fixtures_root = repo_root / "tests" / "fixtures" / "unica_mcp_script_parity"
    snapshot_root = fixtures_root / "cc-1c-skills"
    baseline_path = fixtures_root / "donor-baseline.json"
    provenance_path = repo_root / "docs" / "provenance" / "skill-upstreams.json"
    errors = []
    for path in (baseline_path, provenance_path):
        if not path.is_file():
            errors.append(f"required donor parity contract file is missing: {path}")
    if errors:
        return errors
    baseline = load_json(baseline_path)
    provenance = load_json(provenance_path)
    errors.extend(validate_baseline(snapshot_root, baseline, provenance))
    return errors


def _find_upstream(
    provenance: dict[str, Any], upstream_id: object
) -> dict[str, Any] | None:
    if provenance.get("id") == upstream_id:
        return provenance
    for upstream in provenance.get("upstreams") or []:
        if isinstance(upstream, dict) and upstream.get("id") == upstream_id:
            return upstream
    return None


def _validated_path(
    raw: object,
    label: str,
    field: str,
    errors: list[str],
) -> Path | None:
    if not isinstance(raw, str):
        errors.append(f"{label} {field} must be a string")
        return None
    try:
        return safe_relative_path(raw)
    except ValueError as error:
        errors.append(f"{label} {field}: {error}")
        return None


def _add_script_dependencies(
    snapshot_root: Path,
    raw_script: object,
    dependencies: set[Path],
) -> None:
    if not isinstance(raw_script, str) or not raw_script:
        raise ValueError(f"invalid donor script path: {raw_script!r}")
    relative = safe_relative_path(raw_script)
    if len(relative.parts) != 3 or relative.parts[1] != "scripts":
        raise ValueError(f"unsupported donor script path: {raw_script}")
    scripts_root = snapshot_root / "skills" / relative.parent
    candidates = [snapshot_root / "skills" / relative]
    if relative.suffix == "":
        candidates.extend(
            [
                snapshot_root / "skills" / Path(f"{relative.as_posix()}.py"),
                snapshot_root / "skills" / Path(f"{relative.as_posix()}.ps1"),
            ]
        )
    if not any(candidate.is_file() and not candidate.is_symlink() for candidate in candidates):
        raise FileNotFoundError(f"donor script does not exist: {raw_script}")
    dependencies.update(_regular_files(scripts_root))


def _regular_files(root: Path) -> list[Path]:
    result = []
    if not root.exists():
        return result
    for current, directories, files in os.walk(root, followlinks=False):
        current_path = Path(current)
        directories[:] = [
            name
            for name in directories
            if not (current_path / name).is_symlink()
        ]
        for name in files:
            path = current_path / name
            if path.is_file() and not path.is_symlink():
                result.append(path)
    return sorted(result)


def _snapshot_symlinks(root: Path) -> list[Path]:
    result = []
    if not root.exists():
        return result
    for current, directories, files in os.walk(root, followlinks=False):
        current_path = Path(current)
        for name in [*directories, *files]:
            path = current_path / name
            if path.is_symlink():
                result.append(path)
    return sorted(result)


def _require_regular_file(path: Path) -> None:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"dependency is not a regular file: {path}")
