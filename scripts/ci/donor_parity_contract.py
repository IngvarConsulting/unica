#!/usr/bin/env python3
"""Offline integrity contract for the reviewed cc-1c-skills parity snapshot."""

from __future__ import annotations

import hashlib
import json
import os
import re
from pathlib import Path, PurePosixPath
from typing import Any, Iterable


ALLOWED_RELATIONS = {
    "exact",
    "compatible",
    "unica_extension",
    "platform_override",
    "donor_ahead",
    "intentional_divergence",
}
HEX_40 = re.compile(r"^[0-9a-f]{40}$")
HEX_64 = re.compile(r"^[0-9a-f]{64}$")
OBSERVATION_KEYS = {
    "donorOk",
    "unicaOk",
    "mismatchKind",
    "donorStdoutSha256",
    "unicaStdoutSha256",
    "donorStderrSha256",
    "unicaStderrSha256",
    "donorWorkspaceSha256",
    "unicaWorkspaceSha256",
    "donorExpectedFiles",
    "unicaExpectedFiles",
}
CASE_EXECUTION_PROFILE = {
    "schemaVersion": 1,
    "platformVersion": "8.3.27",
    "exportFormat": "2.20",
    "emptyConfigProjection": {"from": "2.17", "to": "2.20"},
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


def observation_fingerprint(observation: dict[str, Any]) -> str:
    return sha256_json(observation)


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
            "executionProfile": CASE_EXECUTION_PROFILE,
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


def validate_relations(
    repo_root: Path,
    snapshot_root: Path,
    registry: dict[str, Any],
) -> list[str]:
    errors: list[str] = []
    if registry.get("schemaVersion") != 1:
        errors.append("donor relations schemaVersion must be 1")
    relations = registry.get("relations")
    if not isinstance(relations, dict):
        return [*errors, "donor relations must be an object"]

    discovered = set(discover_case_ids(snapshot_root))
    recorded = set(relations)
    for case_id in sorted(discovered - recorded):
        errors.append(f"missing relation for {case_id}")
    for case_id in sorted(recorded - discovered):
        errors.append(f"relation for unknown case {case_id}")

    for case_id in sorted(discovered & recorded):
        relation = relations.get(case_id)
        if not isinstance(relation, dict):
            errors.append(f"relation {case_id} must be an object")
            continue
        if relation.get("caseId") != case_id:
            errors.append(f"relation {case_id} caseId mismatch")
        kind = relation.get("relation")
        if kind not in ALLOWED_RELATIONS:
            errors.append(f"relation {case_id} has invalid kind: {kind!r}")
            continue
        try:
            digest = case_content_digest(snapshot_root, case_id)
        except (FileNotFoundError, ValueError) as error:
            errors.append(f"cannot digest relation case {case_id}: {error}")
            continue
        if relation.get("contentDigest") != digest:
            errors.append(f"relation {case_id} content digest changed")

        if kind == "exact":
            continue
        reason = relation.get("reason")
        if not isinstance(reason, str) or not reason.strip():
            errors.append(f"relation {case_id} reason is required")
        evidence = relation.get("evidence")
        if not isinstance(evidence, list) or not evidence:
            errors.append(f"relation {case_id} evidence is required")
        else:
            for raw in evidence:
                evidence_path = _validated_path(
                    raw, f"relation {case_id}", "evidence", errors
                )
                if evidence_path is not None and not (
                    repo_root / evidence_path
                ).is_file():
                    errors.append(
                        f"relation {case_id} evidence path does not exist: {raw}"
                    )
        observation = relation.get("observation")
        observation_errors = _validate_observation(observation)
        errors.extend(
            f"relation {case_id} {error}" for error in observation_errors
        )
        if isinstance(observation, dict):
            expected = observation_fingerprint(observation)
            if relation.get("observationFingerprint") != expected:
                errors.append(
                    f"relation {case_id} stored observation fingerprint mismatch"
                )
        if kind == "donor_ahead":
            owner = relation.get("owner")
            if not isinstance(owner, str) or not owner.strip():
                errors.append(f"relation {case_id} owner is required")
            if relation.get("decision") not in {"adopt", "defer"}:
                errors.append(
                    f"relation {case_id} decision must be adopt or defer"
                )

    return errors


def validate_relation_observation(
    *,
    relation: dict[str, Any],
    content_digest: str,
    observation: dict[str, Any],
) -> list[str]:
    errors = _validate_observation(observation)
    if relation.get("contentDigest") != content_digest:
        errors.append("donor content digest changed")
    kind = relation.get("relation")
    if kind not in ALLOWED_RELATIONS:
        errors.append(f"invalid relation kind: {kind!r}")
        return errors
    if kind == "exact":
        if not observation_is_exact(observation):
            errors.append("exact relation observed a difference")
        return errors

    fingerprint = observation_fingerprint(observation)
    if relation.get("observationFingerprint") != fingerprint:
        errors.append("observation fingerprint changed")
    return errors


def observation_is_exact(observation: dict[str, Any]) -> bool:
    if observation.get("donorOk") != observation.get("unicaOk"):
        return False
    if observation.get("mismatchKind") not in (None, "exact"):
        return False
    pairs = (
        ("donorStdoutSha256", "unicaStdoutSha256"),
        ("donorStderrSha256", "unicaStderrSha256"),
        ("donorWorkspaceSha256", "unicaWorkspaceSha256"),
        ("donorExpectedFiles", "unicaExpectedFiles"),
    )
    return all(observation.get(left) == observation.get(right) for left, right in pairs)


def validate_repository_contract(repo_root: Path) -> list[str]:
    fixtures_root = repo_root / "tests" / "fixtures" / "unica_mcp_script_parity"
    snapshot_root = fixtures_root / "cc-1c-skills"
    baseline_path = fixtures_root / "donor-baseline.json"
    relations_path = fixtures_root / "donor-relations.json"
    provenance_path = (
        repo_root / "plugins" / "unica" / "provenance" / "skill-upstreams.json"
    )
    errors = []
    for path in (baseline_path, relations_path, provenance_path):
        if not path.is_file():
            errors.append(f"required donor parity contract file is missing: {path}")
    if errors:
        return errors
    baseline = load_json(baseline_path)
    relations = load_json(relations_path)
    provenance = load_json(provenance_path)
    errors.extend(validate_baseline(snapshot_root, baseline, provenance))
    errors.extend(validate_refresh_reviews(repo_root, baseline))
    if relations.get("upstreamId") != baseline.get("upstreamId"):
        errors.append("donor relation upstreamId differs from baseline")
    errors.extend(validate_relations(repo_root, snapshot_root, relations))
    return errors


def validate_refresh_reviews(
    repo_root: Path, baseline: dict[str, Any]
) -> list[str]:
    errors = []
    scopes = baseline.get("scopes")
    if not isinstance(scopes, dict):
        return ["donor baseline scopes must be an object"]
    reviews: dict[str, dict[str, Any]] = {}
    for scope, scope_data in sorted(scopes.items()):
        if not isinstance(scope_data, dict):
            continue
        review_id = scope_data.get("reviewId")
        try:
            review_component = safe_relative_path(review_id)
        except ValueError as error:
            errors.append(f"scope {scope!r} reviewId: {error}")
            continue
        if len(review_component.parts) != 1:
            errors.append(f"scope {scope!r} reviewId must be one path component")
            continue
        if review_id not in reviews:
            review_path = (
                repo_root
                / "plugins"
                / "unica"
                / "provenance"
                / "reviews"
                / f"{review_id}.json"
            )
            if not review_path.is_file():
                errors.append(
                    f"scope {scope!r} refresh review does not exist: "
                    f"{review_path.relative_to(repo_root)}"
                )
                continue
            try:
                reviews[review_id] = load_json(review_path)
            except (ValueError, json.JSONDecodeError) as error:
                errors.append(f"refresh review {review_id!r} is invalid: {error}")
                continue
        review = reviews.get(review_id)
        if review is None:
            continue
        if review.get("reviewStatus") != "reviewed" or review.get("applied") is not True:
            errors.append(f"refresh review {review_id!r} is not reviewed and applied")
        if review.get("targetCommit") != scope_data.get("acceptedCommit"):
            errors.append(
                f"scope {scope!r} accepted commit differs from refresh review "
                f"{review_id!r}"
            )
        affected = review.get("affectedSkills")
        if not isinstance(affected, list) or scope not in affected:
            errors.append(
                f"scope {scope!r} is not covered by refresh review {review_id!r}"
            )
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


def _validate_observation(observation: object) -> list[str]:
    if not isinstance(observation, dict):
        return ["observation must be an object"]
    errors = []
    missing = OBSERVATION_KEYS - set(observation)
    extra = set(observation) - OBSERVATION_KEYS
    if missing:
        errors.append(f"observation is missing fields: {sorted(missing)}")
    if extra:
        errors.append(f"observation has unknown fields: {sorted(extra)}")
    for field in ("donorOk", "unicaOk"):
        if not isinstance(observation.get(field), bool):
            errors.append(f"{field} must be boolean")
    mismatch = observation.get("mismatchKind")
    if mismatch is not None and not isinstance(mismatch, str):
        errors.append("mismatchKind must be a string or null")
    for field in (
        "donorStdoutSha256",
        "unicaStdoutSha256",
        "donorStderrSha256",
        "unicaStderrSha256",
        "donorWorkspaceSha256",
        "unicaWorkspaceSha256",
    ):
        if not HEX_64.fullmatch(str(observation.get(field) or "")):
            errors.append(f"{field} must be a lowercase 64-hex digest")
    for field in ("donorExpectedFiles", "unicaExpectedFiles"):
        value = observation.get(field)
        if not isinstance(value, dict) or not all(
            isinstance(path, str) and isinstance(present, bool)
            for path, present in value.items()
        ):
            errors.append(f"{field} must map paths to booleans")
    return errors
