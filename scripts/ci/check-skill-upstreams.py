#!/usr/bin/env python3
"""Validate and inspect Unica skill upstream provenance."""

from __future__ import annotations

import argparse
import fnmatch
import json
import shutil
import subprocess
from pathlib import Path


DEFAULT_INDEX = Path("plugins/unica/provenance/skill-upstreams.json")
DEFAULT_LOCK = Path("plugins/unica/third-party/tools.lock.json")
DEFAULT_CACHE = Path(".build/skill-upstreams")
ALLOWED_ROLES = {
    "operation-parity",
    "guidance",
    "runtime-tool-contract",
    "tooling",
    "reference",
    "test-fixture",
}
ALLOWED_STATUSES = {
    "adapted",
    "inspiration-only",
    "ported-to-unica",
    "test-fixture-only",
    "still-local-script",
    "superseded-by-unica-mcp",
}
ALLOWED_DECISIONS = {
    "ported",
    "ignored-with-reason",
    "blocked-by-product-contract",
    "needs-tool-update",
    "needs-review",
}


class ValidationReport:
    def __init__(self, errors: list[str] | None = None, warnings: list[str] | None = None) -> None:
        self.errors = errors or []
        self.warnings = warnings or []

    def as_dict(self) -> dict:
        return {"errors": self.errors, "warnings": self.warnings}


class CheckReport:
    def __init__(self, errors: list[str] | None = None, upstreams: list[dict] | None = None) -> None:
        self.errors = errors or []
        self.upstreams = upstreams or []

    def as_dict(self) -> dict:
        return {"errors": self.errors, "upstreams": self.upstreams}


def run_git(args: list[str], cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "commit.gpgsign=false", "-c", "tag.gpgSign=false", *args],
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )


def git_output(args: list[str], cwd: Path) -> str:
    result = subprocess.run(
        ["git", "-c", "commit.gpgsign=false", "-c", "tag.gpgSign=false", *args],
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    return result.stdout.strip()


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def load_locked_tools(lock_file: Path) -> dict[str, dict]:
    if not lock_file.exists():
        return {}
    lock = load_json(lock_file)
    return {tool["name"]: tool for tool in lock.get("tools", [])}


def validate_relative_existing_path(repo_root: Path, rel_path: str, errors: list[str], label: str) -> None:
    path = Path(rel_path)
    if path.is_absolute() or ".." in path.parts:
        errors.append(f"{label}: path must be repository-relative and stay inside repo: {rel_path}")
        return
    if not (repo_root / rel_path).exists():
        errors.append(f"{label}: path does not exist: {rel_path}")


def validate_index(
    repo_root: Path,
    index_file: Path = DEFAULT_INDEX,
    lock_file: Path | None = None,
    **_ignored: object,
) -> ValidationReport:
    lock_file = lock_file or repo_root / DEFAULT_LOCK
    errors: list[str] = []
    warnings: list[str] = []

    if not index_file.exists():
        return ValidationReport(errors=[f"provenance index not found: {index_file}"])

    try:
        data = load_json(index_file)
    except json.JSONDecodeError as exc:
        return ValidationReport(errors=[f"invalid JSON in {index_file}: {exc}"])

    if data.get("schemaVersion") != 1:
        errors.append(f"{index_file}: schemaVersion must be 1")

    upstreams = data.get("upstreams")
    if not isinstance(upstreams, list) or not upstreams:
        errors.append(f"{index_file}: upstreams must be a non-empty list")
        upstreams = []

    locked_tools = load_locked_tools(lock_file)
    skill_root = repo_root / "plugins" / "unica" / "skills"
    local_skills = {
        path.name
        for path in skill_root.iterdir()
        if path.is_dir()
    } if skill_root.exists() else set()

    seen_ids: set[str] = set()
    indexed_skills: set[str] = set()
    for upstream_index, upstream in enumerate(upstreams):
        label = f"upstreams[{upstream_index}]"
        upstream_id = upstream.get("id")
        if not isinstance(upstream_id, str) or not upstream_id:
            errors.append(f"{label}: id is required")
            upstream_id = f"<missing-{upstream_index}>"
        elif upstream_id in seen_ids:
            errors.append(f"{label}: duplicate id: {upstream_id}")
        seen_ids.add(upstream_id)

        for key in ("repository", "trackingRef", "role"):
            if not isinstance(upstream.get(key), str) or not upstream.get(key):
                errors.append(f"{label}: {key} is required")

        role = upstream.get("role")
        if isinstance(role, str) and role not in ALLOWED_ROLES:
            errors.append(f"{label}: unsupported role: {role}")

        tool_lock_ref = upstream.get("toolLockRef")
        if tool_lock_ref:
            if tool_lock_ref not in locked_tools:
                errors.append(f"{label}: toolLockRef not found in tools lock: {tool_lock_ref}")
            if "baselineCommit" in upstream:
                errors.append(f"{label}: baselineCommit must not be set when toolLockRef is used")
        elif not isinstance(upstream.get("baselineCommit"), str) or not upstream.get("baselineCommit"):
            errors.append(f"{label}: baselineCommit is required when toolLockRef is not used")

        entries = upstream.get("entries")
        if not isinstance(entries, list) or not entries:
            errors.append(f"{label}: entries must be a non-empty list")
            continue

        for entry_index, entry in enumerate(entries):
            entry_label = f"{label}.entries[{entry_index}]"
            for key in ("skill", "status", "notes"):
                if not isinstance(entry.get(key), str) or not entry.get(key):
                    errors.append(f"{entry_label}: {key} is required")
            if isinstance(entry.get("skill"), str) and entry.get("skill"):
                indexed_skills.add(entry["skill"])
            status = entry.get("status")
            if isinstance(status, str) and status not in ALLOWED_STATUSES:
                errors.append(f"{entry_label}: unsupported status: {status}")
            decision = entry.get("decision")
            if decision is not None:
                if decision not in ALLOWED_DECISIONS:
                    errors.append(f"{entry_label}: unsupported decision: {decision}")
                if decision == "ignored-with-reason" and not entry.get("decisionReason"):
                    errors.append(f"{entry_label}: decisionReason is required for {decision}")
            if upstream.get("toolLockRef") and "baselineCommit" in entry:
                errors.append(f"{entry_label}: baselineCommit must not be set on entries for toolLockRef upstreams")
            if "baselineCommit" in entry and not isinstance(entry.get("baselineCommit"), str):
                errors.append(f"{entry_label}: baselineCommit must be a string when set")
            if "primarySource" in entry and not isinstance(entry.get("primarySource"), str):
                errors.append(f"{entry_label}: primarySource must be a string when set")

            for key in ("localPaths", "upstreamPaths", "contractPaths"):
                if key in entry and not isinstance(entry[key], list):
                    errors.append(f"{entry_label}: {key} must be a list")
            for key in ("localPaths", "contractPaths"):
                for rel_path in entry.get(key, []):
                    if not isinstance(rel_path, str):
                        errors.append(f"{entry_label}: {key} item must be a string")
                    else:
                        validate_relative_existing_path(repo_root, rel_path, errors, f"{entry_label}.{key}")

            if not entry.get("localPaths") and not entry.get("contractPaths"):
                warnings.append(f"{entry_label}: entry has no localPaths or contractPaths")

    if local_skills:
        for skill in sorted(local_skills - indexed_skills):
            errors.append(f"{index_file}: missing provenance entry for skill: {skill}")
        for skill in sorted(indexed_skills - local_skills):
            errors.append(f"{index_file}: provenance entry does not match a packaged skill: {skill}")

    return ValidationReport(errors=errors, warnings=warnings)


def clone_or_fetch(repository: str, upstream_id: str, cache_dir: Path) -> Path:
    cache_dir.mkdir(parents=True, exist_ok=True)
    repo_dir = cache_dir / upstream_id
    if (repo_dir / ".git").exists():
        run_git(["fetch", "--tags", "--prune", "origin"], cwd=repo_dir)
    else:
        shutil.rmtree(repo_dir, ignore_errors=True)
        run_git(["clone", repository, str(repo_dir)], cwd=cache_dir)
        run_git(["fetch", "--tags", "origin"], cwd=repo_dir)
    return repo_dir


def resolve_ref(repo_dir: Path, ref: str) -> str:
    candidates = [f"origin/{ref}", ref, f"refs/tags/{ref}"]
    last_error = ""
    for candidate in dict.fromkeys(candidates):
        try:
            return git_output(["rev-parse", f"{candidate}^{{}}"], cwd=repo_dir)
        except subprocess.CalledProcessError as exc:
            last_error = (exc.stderr or exc.stdout or "").strip()
    raise RuntimeError(f"failed to resolve {ref}: {last_error}")


def list_changed_paths(repo_dir: Path, baseline: str, target: str) -> list[str]:
    output = git_output(["diff", "--name-only", baseline, target], cwd=repo_dir)
    return [line for line in output.splitlines() if line.strip()]


def path_matches(path: str, patterns: list[str]) -> bool:
    if not patterns:
        return True
    for pattern in patterns:
        if path == pattern or path.startswith(pattern.rstrip("/") + "/") or fnmatch.fnmatch(path, pattern):
            return True
    return False


def latest_semverish_tag(repo_dir: Path) -> str | None:
    output = git_output(["tag", "--list", "v*", "--sort=version:refname"], cwd=repo_dir)
    tags = [line for line in output.splitlines() if line.strip()]
    return tags[-1] if tags else None


def baseline_for(upstream: dict, locked_tools: dict[str, dict]) -> tuple[str, str, str | None]:
    tool_lock_ref = upstream.get("toolLockRef")
    if tool_lock_ref:
        locked = locked_tools[tool_lock_ref]
        return locked["sourceCommit"], f"toolLockRef:{tool_lock_ref}", locked.get("sourceTag")
    return upstream["baselineCommit"], "baselineCommit", upstream.get("baselineTag")


def baseline_for_entry(upstream: dict, entry: dict, locked_tools: dict[str, dict]) -> tuple[str, str, str | None]:
    if "baselineCommit" in entry and not upstream.get("toolLockRef"):
        return entry["baselineCommit"], "entryBaselineCommit", upstream.get("baselineTag")
    return baseline_for(upstream, locked_tools)


def upstream_report(upstream: dict, repo_dir: Path, baseline: str, baseline_source: str) -> dict:
    locked_tools = upstream.get("_lockedTools", {})
    target_commit = resolve_ref(repo_dir, upstream["trackingRef"])
    changed_paths = list_changed_paths(repo_dir, baseline, target_commit)
    watched_changes_by_path: set[str] = set()
    entry_reports = []
    for entry in upstream["entries"]:
        entry_baseline, entry_baseline_source, _entry_baseline_tag = baseline_for_entry(
            upstream,
            entry,
            locked_tools,
        )
        entry_changed_all_paths = list_changed_paths(repo_dir, entry_baseline, target_commit)
        entry_changed_paths = [
            path
            for path in entry_changed_all_paths
            if path_matches(path, entry.get("upstreamPaths", []))
        ]
        if entry.get("primarySource") == "unica" and entry.get("decision") == "ignored-with-reason":
            entry_changed_paths = []
        watched_changes_by_path.update(entry_changed_paths)
        entry_report = {
            "skill": entry["skill"],
            "status": entry["status"],
            "baseline": entry_baseline,
            "baselineSource": entry_baseline_source,
            "decision": entry.get("decision", "needs-review" if entry_changed_paths else "ported"),
            "upstreamDrift": bool(entry_changed_paths),
            "changedPaths": entry_changed_paths,
        }
        if "primarySource" in entry:
            entry_report["primarySource"] = entry["primarySource"]
        entry_reports.append(entry_report)

    latest_tag = latest_semverish_tag(repo_dir)
    baseline_tag = upstream.get("baselineTag")
    watched_changes = sorted(watched_changes_by_path)
    return {
        "id": upstream["id"],
        "role": upstream["role"],
        "repository": upstream["repository"],
        "trackingRef": upstream["trackingRef"],
        "baseline": baseline,
        "baselineSource": baseline_source,
        "targetCommit": target_commit,
        "commitsSinceBaseline": int(git_output(["rev-list", "--count", f"{baseline}..{target_commit}"], cwd=repo_dir)),
        "changedPathCount": len(changed_paths),
        "changedWatchedPathCount": len(watched_changes),
        "changedPaths": watched_changes,
        "affectedEntries": sorted({entry["skill"] for entry in entry_reports if entry["upstreamDrift"]}),
        "upstreamDrift": bool(watched_changes),
        "contractDrift": bool(watched_changes),
        "entries": entry_reports,
        "latestTag": latest_tag,
        "baselineTag": baseline_tag,
        "upstreamTagDrift": bool(
            upstream.get("role") == "runtime-tool-contract"
            and baseline_tag
            and latest_tag
            and latest_tag != baseline_tag
        ),
    }


def check_upstreams(
    repo_root: Path,
    index_file: Path = DEFAULT_INDEX,
    cache_dir: Path | None = None,
    lock_file: Path | None = None,
    **_ignored: object,
) -> CheckReport:
    lock_file = lock_file or repo_root / DEFAULT_LOCK
    cache_dir = cache_dir or repo_root / DEFAULT_CACHE
    validation = validate_index(repo_root, index_file, lock_file)
    if validation.errors:
        return CheckReport(errors=validation.errors)

    data = load_json(index_file)
    locked_tools = load_locked_tools(lock_file)
    reports: list[dict] = []
    errors: list[str] = []

    for upstream in data["upstreams"]:
        try:
            repo_dir = clone_or_fetch(upstream["repository"], upstream["id"], cache_dir)
            baseline, baseline_source, baseline_tag = baseline_for(upstream, locked_tools)
            upstream["_lockedTools"] = locked_tools
            report = upstream_report(upstream, repo_dir, baseline, baseline_source)
            report["baselineTag"] = baseline_tag
            reports.append(report)
        except Exception as exc:  # noqa: BLE001 - CLI should collect every upstream error.
            errors.append(f"{upstream.get('id', '<unknown>')}: {exc}")

    return CheckReport(errors=errors, upstreams=reports)


def prepare_upstream_review(
    repo_root: Path,
    index_file: Path = DEFAULT_INDEX,
    cache_dir: Path | None = None,
    lock_file: Path | None = None,
    review_id: str = "2026-06-15-upstream-review",
) -> dict:
    report = check_upstreams(repo_root, index_file, cache_dir, lock_file)
    if report.errors:
        raise RuntimeError("; ".join(report.errors))
    return {
        "schemaVersion": 1,
        "id": review_id,
        "generatedAt": "2026-06-15",
        "purpose": (
            "Manual upstream drift review. The report starts from the donor baseline that matches "
            "the last local Unica skill adaptation, not from the current donor head."
        ),
        "upstreams": [
            {
                **upstream,
                "reviewStatus": "needs-review" if upstream["upstreamDrift"] else "no-upstream-drift",
            }
            for upstream in report.upstreams
        ],
    }


def print_validation(report: ValidationReport, *, as_json: bool) -> None:
    if as_json:
        print(json.dumps(report.as_dict(), ensure_ascii=False, indent=2))
        return

    if report.errors:
        print("Skill upstream validation failed:")
        for error in report.errors:
            print(f"- {error}")
    else:
        print("Skill upstream validation passed")
    for warning in report.warnings:
        print(f"warning: {warning}")


def print_check(report: CheckReport, *, as_json: bool) -> None:
    if as_json:
        print(json.dumps(report.as_dict(), ensure_ascii=False, indent=2))
        return

    if report.errors:
        print("Skill upstream check failed:")
        for error in report.errors:
            print(f"- {error}")
        return

    for upstream in report.upstreams:
        print(f"{upstream['id']} ({upstream['role']})")
        print(f"  baseline: {upstream['baselineSource']} {upstream['baseline']}")
        print(f"  target:   {upstream['trackingRef']} {upstream['targetCommit']}")
        print(f"  commits since baseline: {upstream['commitsSinceBaseline']}")
        if upstream["upstreamTagDrift"]:
            print(f"  upstream tag drift: {upstream['baselineTag']} -> {upstream['latestTag']}")
        if upstream["changedPaths"]:
            print(f"  changed watched paths: {len(upstream['changedPaths'])}")
            if upstream["affectedEntries"]:
                print("  affected entries: " + ", ".join(upstream["affectedEntries"]))
        else:
            print("  changed watched paths: none")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument("--index-file", type=Path, default=DEFAULT_INDEX)
    parser.add_argument("--lock-file", type=Path, default=DEFAULT_LOCK)
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--validate-only", action="store_true")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--prepare-baseline-review", action="store_true")
    parser.add_argument("--prepare-upstream-review", action="store_true")
    parser.add_argument("--review-id", default="2026-06-15-upstream-review")
    parser.add_argument("--format", choices=["text", "json"], default="text")
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    index_file = args.index_file if args.index_file.is_absolute() else repo_root / args.index_file
    lock_file = args.lock_file if args.lock_file.is_absolute() else repo_root / args.lock_file
    cache_dir = args.cache_dir if args.cache_dir.is_absolute() else repo_root / args.cache_dir
    as_json = args.format == "json"

    if args.prepare_baseline_review or args.prepare_upstream_review:
        review = prepare_upstream_review(repo_root, index_file, cache_dir, lock_file, args.review_id)
        print(json.dumps(review, ensure_ascii=False, indent=2))
        return

    if args.check:
        check_report = check_upstreams(repo_root, index_file, cache_dir, lock_file)
        print_check(check_report, as_json=as_json)
        raise SystemExit(1 if check_report.errors else 0)

    validation = validate_index(repo_root, index_file, lock_file)
    print_validation(validation, as_json=as_json)
    raise SystemExit(1 if validation.errors else 0)


if __name__ == "__main__":
    main()
