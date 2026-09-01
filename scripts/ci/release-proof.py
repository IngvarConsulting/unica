#!/usr/bin/env python3
"""Validate the machine-readable proof boundary for the v0.13 RC package."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections.abc import Mapping
from pathlib import Path
from typing import Any


SCHEMA_VERSION = 1
BASELINE_TAG = "v0.12.3"
TARGETS = ("darwin-arm64", "linux-x64", "win-x64")
LIFECYCLE_SCENARIOS = (
    "fresh_install",
    "upgrade",
    "offline_prefetch",
    "restart",
    "rollback",
)
NATIVE_TOOLS = frozenset(
    {
        "unica.view",
        "unica.apply",
        "unica.find",
        "unica.search",
        "unica.check",
        "unica.diff",
        "unica.run",
        "unica.docs",
    }
)
COMPATIBILITY_TOOLS = NATIVE_TOOLS | frozenset(
    {"unica.task.get", "unica.task.result", "unica.task.cancel"}
)
HEX_40 = re.compile(r"^[0-9a-f]{40}$")
HEX_64 = re.compile(r"^[0-9a-f]{64}$")


class ProofError(ValueError):
    """A release proof input is malformed or violates a release gate."""


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise ProofError(f"cannot hash package file {path}: {error}") from error
    return digest.hexdigest()


def tree_sha256(root: Path) -> str:
    if not root.is_dir():
        raise ProofError(f"downloaded package directory is missing: {root}")
    digest = hashlib.sha256()
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        digest.update(path.relative_to(root).as_posix().encode("utf-8"))
        digest.update(b"\0")
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()


def read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ProofError(f"cannot read proof input {path}: {error}") from error
    if not isinstance(value, dict):
        raise ProofError(f"proof input must be a JSON object: {path}")
    return value


def _require_string(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value:
        raise ProofError(f"{name} must be a non-empty string")
    return value


def _validate_wire(profile: str, evidence: dict[str, Any]) -> dict[str, Any]:
    if evidence.get("schemaVersion") != SCHEMA_VERSION:
        raise ProofError(f"{profile} wire evidence schemaVersion must be {SCHEMA_VERSION}")
    if evidence.get("profile") != profile:
        raise ProofError(f"{profile} wire evidence has wrong profile")
    names = evidence.get("toolNames")
    if not isinstance(names, list) or not all(isinstance(name, str) and name for name in names):
        raise ProofError(f"{profile} wire evidence toolNames must be a string array")
    if len(names) != len(set(names)):
        raise ProofError(f"{profile} wire evidence contains duplicate tool names")
    actual = set(names)
    expected = NATIVE_TOOLS if profile == "native" else COMPATIBILITY_TOOLS
    if actual != expected:
        missing = sorted(expected - actual)
        unexpected = sorted(actual - expected)
        detail = []
        if missing:
            detail.append("missing: " + ", ".join(missing))
        if unexpected:
            detail.append("unexpected: " + ", ".join(unexpected))
        raise ProofError(f"{profile} wire surface differs: {'; '.join(detail)}")
    if evidence.get("toolCount") != len(names):
        raise ProofError(f"{profile} wire evidence toolCount disagrees with toolNames")
    server_info = evidence.get("serverInfo")
    if server_info is None and profile == "compatibility":
        raise ProofError(f"{profile} wire evidence must identify server unica")
    if server_info is not None and (
        not isinstance(server_info, dict) or server_info.get("name") != "unica"
    ):
        raise ProofError(f"{profile} wire evidence must identify server unica")
    protocol = _require_string(evidence.get("protocolVersion"), f"{profile} protocolVersion")
    negotiated = _require_string(
        evidence.get("serverProtocolVersion"), f"{profile} serverProtocolVersion"
    )
    if negotiated != protocol:
        raise ProofError(
            f"{profile} negotiated protocol {negotiated} differs from requested {protocol}"
        )
    tasks = evidence.get("tasksCapability")
    if profile == "native" and (protocol != "2026-07-28" or tasks != "on"):
        raise ProofError("native profile must use protocol 2026-07-28 with Tasks on")
    if profile == "compatibility" and (protocol != "2025-06-18" or tasks != "off"):
        raise ProofError("compatibility profile must use protocol 2025-06-18 with Tasks off")
    return {
        "toolCount": len(names),
        "toolNames": sorted(names),
        "protocolVersion": protocol,
        "serverProtocolVersion": negotiated,
        "tasksCapability": tasks,
    }


def _validate_wire_profiles(
    profile: str, wires: Mapping[str, dict[str, Any]]
) -> dict[str, Any]:
    if set(wires) != set(TARGETS):
        raise ProofError(f"{profile} wire evidence must cover all targets: {', '.join(TARGETS)}")
    validated = {target: _validate_wire(profile, wires[target]) for target in TARGETS}
    first = validated[TARGETS[0]]
    if any(value != first for value in validated.values()):
        raise ProofError(f"{profile} wire evidence differs across targets")
    return {**first, "targets": validated}


def _validate_baseline(baseline: dict[str, Any]) -> tuple[str, set[str]]:
    if baseline.get("schemaVersion") != 1:
        raise ProofError("v0.12.3 baseline schemaVersion must be 1")
    source = baseline.get("source")
    wire = baseline.get("wire")
    if not isinstance(source, dict) or source.get("tag") != BASELINE_TAG:
        raise ProofError(f"legacy baseline must identify {BASELINE_TAG}")
    if not isinstance(wire, dict) or wire.get("toolCount") != 74:
        raise ProofError("legacy baseline must contain the observed 74-tool surface")
    names = wire.get("toolNames")
    if (
        not isinstance(names, list)
        or len(names) != 74
        or not all(isinstance(name, str) and name for name in names)
        or len(set(names)) != 74
    ):
        raise ProofError("legacy baseline must contain 74 unique tool names")
    return str(source["tag"]), set(names)


def _validate_package(
    package: dict[str, Any], package_dir: Path, expected_source_commit: str
) -> dict[str, Any]:
    if package.get("schemaVersion") != SCHEMA_VERSION:
        raise ProofError(f"package evidence schemaVersion must be {SCHEMA_VERSION}")
    version = _require_string(package.get("pluginVersion"), "package pluginVersion")
    source_commit = _require_string(package.get("sourceCommit"), "package sourceCommit")
    if not HEX_40.fullmatch(source_commit):
        raise ProofError("package sourceCommit must be 40 lowercase hexadecimal characters")
    for key in ("packageSha256", "runtimeManifestSha256"):
        value = _require_string(package.get(key), f"package {key}")
        if not HEX_64.fullmatch(value):
            raise ProofError(f"package {key} must be 64 lowercase hexadecimal characters")
    if package.get("versionBumped") is not False:
        raise ProofError("P0 package proof must not bump the version")
    if package.get("published") is not False:
        raise ProofError("P0 package proof must not publish")
    if package.get("tag") is not None:
        raise ProofError("P0 package proof must not create a tag")
    if source_commit != expected_source_commit:
        raise ProofError("package sourceCommit does not match checked-out source")
    plugin_dir = package_dir / "plugins" / "unica"
    manifest_paths = [
        plugin_dir / ".codex-plugin" / "plugin.json",
        plugin_dir / ".claude-plugin" / "plugin.json",
    ]
    versions: list[str] = []
    for manifest_path in manifest_paths:
        manifest = read_json(manifest_path)
        versions.append(_require_string(manifest.get("version"), "packaged plugin version"))
    if versions != [version, version]:
        raise ProofError("packaged host manifest versions do not match package evidence")
    runtime_manifest = plugin_dir / "runtime-manifest.json"
    if not runtime_manifest.is_file():
        raise ProofError("packaged runtime manifest is missing")
    if package["packageSha256"] != tree_sha256(package_dir):
        raise ProofError("packageSha256 does not match downloaded package")
    if package["runtimeManifestSha256"] != file_sha256(runtime_manifest):
        raise ProofError("runtimeManifestSha256 does not match downloaded package")
    return {
        "pluginVersion": version,
        "sourceCommit": source_commit,
        "packageSha256": package["packageSha256"],
        "runtimeManifestSha256": package["runtimeManifestSha256"],
    }


def _validate_asset_reports(asset_dir: Path, expected_plugin_version: str) -> dict[str, Any]:
    if not asset_dir.is_dir():
        raise ProofError(f"asset verification evidence directory is missing: {asset_dir}")
    paths = sorted(asset_dir.glob("asset-verification-*.json"))
    if len(paths) != len(TARGETS):
        raise ProofError("asset verification evidence must contain all targets")
    reports: list[dict[str, Any]] = []
    seen_targets: set[str] = set()
    for path in paths:
        report = read_json(path)
        if report.get("schemaVersion") != SCHEMA_VERSION or report.get("status") != "passed":
            raise ProofError(f"asset verification report is not passed: {path.name}")
        if report.get("source") != "local-build":
            raise ProofError(f"asset verification report has wrong source: {path.name}")
        if report.get("pluginVersion") != expected_plugin_version:
            raise ProofError(f"asset verification pluginVersion does not match: {path.name}")
        targets = report.get("targets")
        if not isinstance(targets, list) or len(targets) != 1 or targets[0] not in TARGETS:
            raise ProofError(f"asset verification report has invalid targets: {path.name}")
        target = targets[0]
        if target in seen_targets:
            raise ProofError(f"duplicate asset verification target: {target}")
        checks = report.get("checks")
        if not isinstance(checks, dict) or set(checks) != {
            "artifactSet", "archiveChecksum", "memberChecksums", "memberMetadata"
        } or not all(isinstance(value, bool) and value for value in checks.values()):
            raise ProofError(f"asset verification checks are incomplete: {path.name}")
        seen_targets.add(target)
        reports.append({"target": target, "evidence": path.name})
    if seen_targets != set(TARGETS):
        raise ProofError("asset verification evidence must cover all targets")
    return {"status": "passed", "source": "local-build", "targets": reports}


def _validate_lifecycle(assessment: dict[str, Any], mode: str) -> dict[str, dict[str, Any]]:
    if assessment.get("schemaVersion") != SCHEMA_VERSION:
        raise ProofError(f"assessment schemaVersion must be {SCHEMA_VERSION}")
    summary = assessment.get("summary")
    if not isinstance(summary, dict) or summary.get("status") != "passed":
        raise ProofError("release assessment summary must be passed")
    lifecycle = assessment.get("lifecycle")
    if not isinstance(lifecycle, dict) or set(lifecycle) != set(LIFECYCLE_SCENARIOS):
        raise ProofError("assessment must contain separate outcomes for all lifecycle scenarios")
    result: dict[str, dict[str, Any]] = {}
    for scenario in LIFECYCLE_SCENARIOS:
        outcome = lifecycle[scenario]
        if not isinstance(outcome, dict):
            raise ProofError(f"{scenario} lifecycle outcome must be an object")
        status = outcome.get("status")
        if status not in {"passed", "deferred", "failed"}:
            raise ProofError(f"{scenario} lifecycle outcome has invalid status: {status!r}")
        supported = outcome.get("supported")
        if not isinstance(supported, bool):
            raise ProofError(f"{scenario} lifecycle outcome must declare supported boolean")
        if status == "passed" and not supported:
            raise ProofError(f"{scenario} passed lifecycle outcome must be supported")
        evidence = outcome.get("evidence")
        if not isinstance(evidence, list) or not evidence or not all(
            isinstance(item, str) and item for item in evidence
        ):
            raise ProofError(f"{scenario} lifecycle outcome must name machine-readable evidence")
        if status == "failed" or (mode == "rc" and status != "passed"):
            raise ProofError(f"{scenario} lifecycle outcome is {status} in {mode} mode")
        result[scenario] = {
            "status": status,
            "supported": supported,
            "evidence": list(evidence),
        }
    return result


def evaluate_proof(
    *,
    native_wires: Mapping[str, dict[str, Any]],
    compatibility_wires: Mapping[str, dict[str, Any]],
    baseline: dict[str, Any],
    assessment: dict[str, Any],
    package: dict[str, Any],
    package_dir: Path,
    asset_verification_dir: Path,
    source_commit: str,
    release_tag: str,
    mode: str = "dry",
) -> dict[str, Any]:
    if mode not in {"dry", "rc"}:
        raise ProofError(f"unsupported proof mode: {mode}")
    release_tag = _require_string(release_tag, "releaseTag")
    native = _validate_wire_profiles("native", native_wires)
    compatibility = _validate_wire_profiles("compatibility", compatibility_wires)
    baseline_tag, legacy_names = _validate_baseline(baseline)
    package_summary = _validate_package(package, package_dir, source_commit)
    if release_tag != "v" + package_summary["pluginVersion"]:
        raise ProofError("releaseTag does not match package pluginVersion")
    assets = _validate_asset_reports(
        asset_verification_dir, package_summary["pluginVersion"]
    )
    all_rc_names = {
        name
        for surface in (native, compatibility)
        for target in surface["targets"].values()
        for name in target["toolNames"]
    }
    overlap = sorted(all_rc_names & legacy_names)
    if overlap:
        raise ProofError("legacy baseline overlap: " + ", ".join(overlap))
    lifecycle = _validate_lifecycle(assessment, mode)

    return {
        "schemaVersion": SCHEMA_VERSION,
        "mode": mode,
        "status": "passed",
        "releaseTag": release_tag,
        "package": package_summary,
        "surfaces": {"native": native, "compatibility": compatibility},
        "legacyBaseline": {
            "tag": baseline_tag,
            "toolCount": len(legacy_names),
            "overlap": overlap,
        },
        "lifecycle": lifecycle,
        "assets": assets,
        "promotion": {
            "releaseTag": release_tag,
            "promote": False,
            "reason": "P0 proof is never a publication or promotion action",
        },
        "guards": {"noVersionBump": True, "noTag": True, "noPublication": True},
    }


def render_summary(report: dict[str, Any]) -> str:
    lines = [
        "# Unica P0 RC/package proof",
        "",
        f"- Status: `{report['status']}`",
        f"- Mode: `{report['mode']}`",
        f"- Release tag input: `{report['releaseTag']}`",
        f"- Native surface: `{report['surfaces']['native']['toolCount']}` tools",
        f"- Compatibility surface: `{report['surfaces']['compatibility']['toolCount']}` tools",
        f"- Legacy overlap: `{len(report['legacyBaseline']['overlap'])}`",
        "",
        "## Lifecycle outcomes",
        "",
        "| Scenario | Status | Evidence |",
        "| --- | --- | --- |",
    ]
    for scenario in LIFECYCLE_SCENARIOS:
        outcome = report["lifecycle"][scenario]
        lines.append(f"| `{scenario}` | `{outcome['status']}` | {', '.join(outcome['evidence'])} |")
    lines.extend(
        [
            "",
            "P0 guards: no version bump, tag, release publication, or marketplace promotion.",
            "",
        ]
    )
    return "\n".join(lines)


def write_report(report: dict[str, Any], out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "release-proof.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (out_dir / "release-proof.md").write_text(render_summary(report), encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wire-dir", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--assessment", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--package-dir", type=Path, required=True)
    parser.add_argument("--asset-verification-dir", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--release-tag", required=True)
    parser.add_argument("--mode", choices=("dry", "rc"), default="dry")
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        native_wires = {
            target: read_json(args.wire_dir / f"native-{target}.json") for target in TARGETS
        }
        compatibility_wires = {
            target: read_json(args.wire_dir / f"compatibility-{target}.json")
            for target in TARGETS
        }
        report = evaluate_proof(
            native_wires=native_wires,
            compatibility_wires=compatibility_wires,
            baseline=read_json(args.baseline),
            assessment=read_json(args.assessment),
            package=read_json(args.package),
            package_dir=args.package_dir,
            asset_verification_dir=args.asset_verification_dir,
            source_commit=args.source_commit,
            release_tag=args.release_tag,
            mode=args.mode,
        )
    except ProofError as error:
        raise SystemExit(f"P0 release proof failed: {error}") from error
    write_report(report, args.out_dir)
    print(f"P0 release proof passed: {args.out_dir / 'release-proof.json'}")


if __name__ == "__main__":
    main()
