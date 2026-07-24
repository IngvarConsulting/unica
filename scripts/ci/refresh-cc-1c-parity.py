#!/usr/bin/env python3
"""Prepare and atomically apply a reviewed cc-1c-skills parity refresh."""

from __future__ import annotations

import argparse
import copy
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Iterable

import donor_parity_contract as contract


CASE_SCOPE_OWNERS = {
    "cfe-borrow": "cfe-borrow",
    "form-compile": "form-compile",
    "form-compile-from-object": "form-compile",
    "meta-compile": "meta-compile",
    "skd-compile": "dcs-compile",
}
DONOR_SKILL_OWNERS = {
    "skd-compile": "dcs-compile",
    "skd-edit": "dcs-edit",
    "skd-info": "dcs-info",
    "skd-validate": "dcs-validate",
}
UPSTREAM_ID = "cc-1c-skills"
FIXTURES_RELATIVE = Path("tests/fixtures/unica_mcp_script_parity")
SNAPSHOT_NAME = "cc-1c-skills"
BASELINE_NAME = "donor-baseline.json"
RELATIONS_NAME = "donor-relations.json"
PROVENANCE_RELATIVE = Path("plugins/unica/provenance/skill-upstreams.json")
REVIEWS_RELATIVE = Path("plugins/unica/provenance/reviews")


class RefreshError(RuntimeError):
    pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Prepare or apply a reviewed cc-1c-skills parity refresh."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    prepare = subparsers.add_parser("prepare")
    prepare.add_argument("--repo-root", type=Path, required=True)
    prepare.add_argument("--upstream-cache", type=Path, required=True)
    prepare.add_argument("--target", required=True)
    prepare.add_argument("--review-id", required=True)
    prepare.add_argument("--skill", action="append", default=[])
    prepare.add_argument("--full-corpus", action="store_true")

    apply = subparsers.add_parser("apply")
    apply.add_argument("--repo-root", type=Path, required=True)
    apply.add_argument("--review", type=Path, required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    try:
        if args.command == "prepare":
            review_path = prepare_refresh(
                repo_root=args.repo_root.resolve(),
                upstream_cache=args.upstream_cache.resolve(),
                target=args.target,
                review_id=args.review_id,
                selected_skills=args.skill,
                full_corpus=args.full_corpus,
            )
            print(review_path)
        else:
            tracked_review = apply_refresh(
                repo_root=args.repo_root.resolve(),
                review_path=args.review.resolve(),
            )
            print(tracked_review)
        return 0
    except (
        RefreshError,
        FileNotFoundError,
        ValueError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


def prepare_refresh(
    *,
    repo_root: Path,
    upstream_cache: Path,
    target: str,
    review_id: str,
    selected_skills: list[str],
    full_corpus: bool = False,
) -> Path:
    _validate_review_id(review_id)
    _require_git_repository(upstream_cache)
    provenance_path = repo_root / PROVENANCE_RELATIVE
    provenance = contract.load_json(provenance_path)
    upstream = _find_upstream(provenance)
    available_owners = {
        entry.get("skill")
        for entry in upstream.get("entries") or []
        if isinstance(entry, dict)
    }

    maybe_fetch(upstream_cache)
    target_commit = resolve_target(upstream_cache, target)
    accepted_snapshot = repo_root / FIXTURES_RELATIVE / SNAPSHOT_NAME
    target_case_scopes = _available_case_scopes(upstream_cache, target_commit)
    accepted_case_scopes = {
        case_id.split("/", 1)[0]
        for case_id in contract.discover_case_ids(accepted_snapshot)
    }
    relevant_case_scopes = target_case_scopes | accepted_case_scopes
    selectable_owners = {
        owner
        for scope, owner in CASE_SCOPE_OWNERS.items()
        if scope in relevant_case_scopes and owner in available_owners
    }
    if selected_skills:
        unknown = sorted(set(selected_skills) - selectable_owners)
        if unknown:
            raise RefreshError(
                f"unknown selected skill(s): {', '.join(unknown)}"
            )
        requested_owners = set(selected_skills)
    else:
        requested_owners = selectable_owners
    if not requested_owners:
        raise RefreshError("no donor parity skills were selected")
    selected_case_scopes = {
        scope
        for scope, owner in CASE_SCOPE_OWNERS.items()
        if owner in requested_owners and scope in relevant_case_scopes
    }

    refresh_root = (
        repo_root / ".build" / "donor-parity-refresh" / review_id
    )
    candidate_root = refresh_root / "candidate"
    staging_root = refresh_root / "candidate.tmp"
    if staging_root.exists():
        shutil.rmtree(staging_root)
    staging_root.mkdir(parents=True)
    candidate_snapshot = staging_root / SNAPSHOT_NAME
    copied_upstream_paths: set[str] = set()
    corpus_skill_names: set[str] = set()
    if full_corpus:
        candidate_snapshot.mkdir()
        corpus_skill_names, copied = copy_full_corpus(
            upstream_cache,
            target_commit,
            candidate_snapshot,
        )
        copied_upstream_paths.update(copied)
    elif accepted_snapshot.is_dir():
        _reject_filesystem_symlinks(accepted_snapshot)
        shutil.copytree(accepted_snapshot, candidate_snapshot)
    else:
        candidate_snapshot.mkdir()

    if not full_corpus:
        for scope in sorted(selected_case_scopes):
            destination = candidate_snapshot / "cases" / scope
            if destination.exists():
                shutil.rmtree(destination)
            copied = copy_case_scope(
                upstream_cache,
                target_commit,
                scope,
                candidate_snapshot,
            )
            copied_upstream_paths.update(copied)
            _validate_watched_paths(upstream, CASE_SCOPE_OWNERS[scope], copied)

    dependency_donor_skills = _case_dependency_skills(
        candidate_snapshot, selected_case_scopes
    )
    affected_skills = requested_owners | {
        donor_skill_owner(skill) for skill in dependency_donor_skills
    }
    donor_skills_to_copy = set(dependency_donor_skills)
    for owner in requested_owners:
        if not any(
            donor_skill_owner(donor_skill) == owner
            for donor_skill in dependency_donor_skills
        ):
            donor_skills_to_copy.add(owner)
    for donor_skill in sorted(donor_skills_to_copy):
        owner = donor_skill_owner(donor_skill)
        if owner not in available_owners:
            raise RefreshError(
                f"donor dependency skill is missing from provenance: "
                f"{donor_skill} -> {owner}"
            )
        if full_corpus:
            continue
        destination = candidate_snapshot / "skills" / donor_skill
        if destination.exists():
            shutil.rmtree(destination)
        copied = copy_skill_scripts(
            upstream_cache,
            target_commit,
            donor_skill,
            candidate_snapshot,
        )
        copied_upstream_paths.update(copied)
        _validate_watched_paths(upstream, owner, copied)

    candidate_provenance = copy.deepcopy(provenance)
    candidate_upstream = _find_upstream(candidate_provenance)
    for entry in candidate_upstream.get("entries") or []:
        if entry.get("skill") in affected_skills:
            entry["parityBaselineCommit"] = target_commit

    old_baseline_path = repo_root / FIXTURES_RELATIVE / BASELINE_NAME
    old_relations_path = repo_root / FIXTURES_RELATIVE / RELATIONS_NAME
    old_baseline = (
        contract.load_json(old_baseline_path)
        if old_baseline_path.is_file()
        else {}
    )
    old_relations_registry = (
        contract.load_json(old_relations_path)
        if old_relations_path.is_file()
        else {
            "schemaVersion": 1,
            "upstreamId": UPSTREAM_ID,
            "relations": {},
        }
    )
    old_relations = old_relations_registry.get("relations") or {}
    if full_corpus:
        candidate_baseline = build_full_corpus_baseline(
            candidate_snapshot,
            candidate_upstream,
            target_commit=target_commit,
            affected_skills=affected_skills,
            executable_case_scopes=selected_case_scopes,
            corpus_skill_names=corpus_skill_names,
            review_id=review_id,
        )
    else:
        candidate_baseline = build_baseline(
            candidate_snapshot,
            candidate_upstream,
            target_commit=target_commit,
            affected_skills=affected_skills,
            old_baseline=old_baseline,
            review_id=review_id,
        )
    baseline_errors = contract.validate_baseline(
        candidate_snapshot, candidate_baseline, candidate_provenance
    )
    if baseline_errors:
        raise RefreshError(
            "candidate baseline is invalid:\n" + "\n".join(baseline_errors)
        )

    previous_corpus_cases = set(contract.discover_case_ids(accepted_snapshot))
    next_corpus_cases = set(contract.discover_case_ids(candidate_snapshot))
    previous_cases = set(
        contract.executable_case_ids(accepted_snapshot, old_baseline)
    )
    next_cases = set(
        contract.executable_case_ids(candidate_snapshot, candidate_baseline)
    )
    old_case_digests = {
        case_id: data.get("contentDigest")
        for case_id, data in (old_baseline.get("cases") or {}).items()
        if isinstance(data, dict)
    }
    new_case_digests = {
        case_id: data["contentDigest"]
        for case_id, data in candidate_baseline["cases"].items()
    }
    added_cases = sorted(next_cases - previous_cases)
    removed_cases = sorted(previous_cases - next_cases)
    changed_cases = sorted(
        case_id
        for case_id in previous_cases & next_cases
        if old_case_digests.get(case_id) != new_case_digests.get(case_id)
    )
    unchanged_cases = sorted(
        case_id
        for case_id in previous_cases & next_cases
        if old_case_digests.get(case_id) == new_case_digests.get(case_id)
    )
    carried_relations = sorted(
        case_id for case_id in unchanged_cases if case_id in old_relations
    )
    needs_review = sorted(
        set(added_cases)
        | set(removed_cases)
        | set(changed_cases)
        | (set(unchanged_cases) - set(carried_relations))
    )
    case_decisions: dict[str, dict[str, Any]] = {
        case_id: {"status": "carried"} for case_id in carried_relations
    }
    for case_id in needs_review:
        if case_id in removed_cases:
            case_decisions[case_id] = {
                "status": "needs-review",
                "decision": None,
            }
        else:
            case_decisions[case_id] = {
                "status": "needs-review",
                "relation": None,
            }

    changed_paths = _changed_snapshot_paths(
        accepted_snapshot, candidate_snapshot
    )
    _write_json(staging_root / BASELINE_NAME, candidate_baseline)
    _write_json(staging_root / "skill-upstreams.json", candidate_provenance)
    if candidate_root.exists():
        shutil.rmtree(candidate_root)
    staging_root.rename(candidate_root)

    review = {
        "schemaVersion": 1,
        "upstreamId": UPSTREAM_ID,
        "reviewId": review_id,
        "reviewStatus": "needs-review",
        "targetRef": target,
        "targetCommit": target_commit,
        "fullCorpus": full_corpus,
        "upstreamCache": _portable_path(repo_root, upstream_cache),
        "candidatePath": candidate_root.relative_to(repo_root).as_posix(),
        "copiedUpstreamPaths": sorted(copied_upstream_paths),
        "selectedSkills": sorted(requested_owners),
        "selectedCaseScopes": sorted(selected_case_scopes),
        "affectedSkills": sorted(affected_skills),
        "previousCommits": {
            scope: data.get("acceptedCommit")
            for scope, data in sorted(
                (old_baseline.get("scopes") or {}).items()
            )
            if isinstance(data, dict)
        },
        "changedPaths": changed_paths,
        "corpusAddedCases": sorted(next_corpus_cases - previous_corpus_cases),
        "corpusRemovedCases": sorted(previous_corpus_cases - next_corpus_cases),
        "addedCases": added_cases,
        "removedCases": removed_cases,
        "changedCases": changed_cases,
        "unchangedCases": unchanged_cases,
        "carriedRelations": carried_relations,
        "needsReview": needs_review,
        "caseDecisions": case_decisions,
        "acceptedBaselineSha256": _optional_sha256(old_baseline_path),
        "acceptedRelationsSha256": _optional_sha256(old_relations_path),
        "acceptedProvenanceSha256": contract.sha256_file(provenance_path),
        "candidateBaselineSha256": contract.sha256_file(
            candidate_root / BASELINE_NAME
        ),
        "candidateProvenanceSha256": contract.sha256_file(
            candidate_root / "skill-upstreams.json"
        ),
        "candidateSnapshotSha256": snapshot_digest(
            candidate_root / SNAPSHOT_NAME
        ),
    }
    review_path = refresh_root / "review.json"
    _write_json(review_path, review)
    return review_path


def apply_refresh(*, repo_root: Path, review_path: Path) -> Path:
    review = contract.load_json(review_path)
    if review.get("schemaVersion") != 1 or review.get("upstreamId") != UPSTREAM_ID:
        raise RefreshError("unsupported donor refresh review")
    if review.get("reviewStatus") != "reviewed":
        raise RefreshError("unresolved donor relations: reviewStatus is not reviewed")

    unresolved = []
    removed = set(review.get("removedCases") or [])
    decisions = review.get("caseDecisions")
    if not isinstance(decisions, dict):
        raise RefreshError("unresolved donor relations: caseDecisions is missing")
    for case_id in review.get("needsReview") or []:
        decision = decisions.get(case_id)
        if not isinstance(decision, dict) or decision.get("status") != "reviewed":
            unresolved.append(case_id)
            continue
        if case_id in removed:
            if decision.get("decision") != "remove":
                unresolved.append(case_id)
        elif not isinstance(decision.get("relation"), dict):
            unresolved.append(case_id)
    if unresolved:
        raise RefreshError(
            "unresolved donor relations: " + ", ".join(sorted(unresolved))
        )

    candidate_relative = contract.safe_relative_path(review["candidatePath"])
    candidate_root = repo_root / candidate_relative
    candidate_snapshot = candidate_root / SNAPSHOT_NAME
    candidate_baseline_path = candidate_root / BASELINE_NAME
    candidate_provenance_path = candidate_root / "skill-upstreams.json"
    _verify_digest(
        candidate_baseline_path,
        review.get("candidateBaselineSha256"),
        "candidate baseline",
    )
    _verify_digest(
        candidate_provenance_path,
        review.get("candidateProvenanceSha256"),
        "candidate provenance",
    )
    if snapshot_digest(candidate_snapshot) != review.get(
        "candidateSnapshotSha256"
    ):
        raise RefreshError("candidate snapshot changed after prepare")

    upstream_cache = _resolve_portable_path(
        repo_root, review.get("upstreamCache")
    )
    resolved = resolve_target(upstream_cache, review["targetRef"])
    if resolved != review.get("targetCommit"):
        raise RefreshError(
            f"target ref moved after review: {review.get('targetCommit')} -> {resolved}"
        )

    fixtures_root = repo_root / FIXTURES_RELATIVE
    accepted_snapshot = fixtures_root / SNAPSHOT_NAME
    baseline_path = fixtures_root / BASELINE_NAME
    relations_path = fixtures_root / RELATIONS_NAME
    provenance_path = repo_root / PROVENANCE_RELATIVE
    _verify_optional_digest(
        baseline_path,
        review.get("acceptedBaselineSha256"),
        "accepted baseline",
    )
    _verify_optional_digest(
        relations_path,
        review.get("acceptedRelationsSha256"),
        "accepted relations",
    )
    _verify_digest(
        provenance_path,
        review.get("acceptedProvenanceSha256"),
        "accepted provenance",
    )

    baseline = contract.load_json(candidate_baseline_path)
    provenance = contract.load_json(candidate_provenance_path)
    current_registry = (
        contract.load_json(relations_path)
        if relations_path.is_file()
        else {
            "schemaVersion": 1,
            "upstreamId": UPSTREAM_ID,
            "relations": {},
        }
    )
    relations = dict(current_registry.get("relations") or {})
    for case_id in removed:
        relations.pop(case_id, None)
    for case_id in review.get("needsReview") or []:
        if case_id not in removed:
            relations[case_id] = decisions[case_id]["relation"]
    registry = {
        "schemaVersion": 1,
        "upstreamId": UPSTREAM_ID,
        "relations": dict(sorted(relations.items())),
    }
    baseline_errors = contract.validate_baseline(
        candidate_snapshot, baseline, provenance
    )
    relation_errors = contract.validate_relations(
        repo_root, candidate_snapshot, registry, baseline
    )
    if baseline_errors or relation_errors:
        raise RefreshError(
            "reviewed candidate is invalid:\n"
            + "\n".join([*baseline_errors, *relation_errors])
        )

    review_id = review.get("reviewId")
    _validate_review_id(review_id)
    tracked_review = repo_root / REVIEWS_RELATIVE / f"{review_id}.json"
    published_review = copy.deepcopy(review)
    published_review["applied"] = True
    _publish_atomically(
        review_root=review_path.parent,
        accepted_snapshot=accepted_snapshot,
        candidate_snapshot=candidate_snapshot,
        files={
            baseline_path: baseline,
            relations_path: registry,
            provenance_path: provenance,
            tracked_review: published_review,
        },
    )
    return tracked_review


def build_baseline(
    snapshot_root: Path,
    upstream: dict[str, Any],
    *,
    target_commit: str,
    affected_skills: set[str],
    old_baseline: dict[str, Any],
    review_id: str,
) -> dict[str, Any]:
    repository = upstream.get("repository")
    tracking_ref = upstream.get("trackingRef")
    old_scopes = old_baseline.get("scopes") or {}
    discovered_cases = contract.discover_case_ids(snapshot_root)
    case_scopes_by_owner: dict[str, set[str]] = {}
    for case_id in discovered_cases:
        case_scope = case_id.split("/", 1)[0]
        owner = CASE_SCOPE_OWNERS.get(case_scope)
        if owner is None:
            raise RefreshError(f"case scope has no explicit owner: {case_scope}")
        case_scopes_by_owner.setdefault(owner, set()).add(case_scope)

    file_records = []
    scopes_with_files: set[str] = set()
    for path in _filesystem_regular_files(snapshot_root):
        local_path = path.relative_to(snapshot_root).as_posix()
        scope, upstream_path = _snapshot_file_source(local_path)
        scopes_with_files.add(scope)
        old_scope = old_scopes.get(scope) or {}
        commit = (
            target_commit
            if scope in affected_skills
            else old_scope.get("acceptedCommit")
        )
        if not isinstance(commit, str) or not contract.HEX_40.fullmatch(commit):
            raise RefreshError(
                f"no concrete accepted commit is available for scope {scope}"
            )
        file_records.append(
            {
                "scope": scope,
                "upstreamPath": upstream_path,
                "localPath": local_path,
                "acceptedCommit": commit,
                "sha256": contract.sha256_file(path),
            }
        )
    file_records.sort(key=lambda item: item["localPath"])

    scope_names = scopes_with_files | affected_skills | set(case_scopes_by_owner)
    scopes = {}
    for scope in sorted(scope_names):
        old_scope = old_scopes.get(scope) or {}
        commit = (
            target_commit
            if scope in affected_skills
            else old_scope.get("acceptedCommit")
        )
        if not isinstance(commit, str) or not contract.HEX_40.fullmatch(commit):
            raise RefreshError(
                f"no concrete accepted commit is available for scope {scope}"
            )
        scopes[scope] = {
            "ownerSkill": scope,
            "caseScopes": sorted(case_scopes_by_owner.get(scope, set())),
            "acceptedCommit": commit,
            "reviewId": (
                review_id
                if scope in affected_skills
                else old_scope.get("reviewId")
            ),
            "contentDigest": contract.scope_content_digest(
                file_records, scope
            ),
        }

    cases = {}
    for case_id in discovered_cases:
        case_scope = case_id.split("/", 1)[0]
        cases[case_id] = {
            "scope": CASE_SCOPE_OWNERS[case_scope],
            "contentDigest": contract.case_content_digest(
                snapshot_root, case_id
            ),
        }
    return {
        "schemaVersion": 1,
        "upstreamId": UPSTREAM_ID,
        "repository": repository,
        "trackingRef": tracking_ref,
        "scopes": scopes,
        "files": file_records,
        "cases": dict(sorted(cases.items())),
    }


def build_full_corpus_baseline(
    snapshot_root: Path,
    upstream: dict[str, Any],
    *,
    target_commit: str,
    affected_skills: set[str],
    executable_case_scopes: set[str],
    corpus_skill_names: set[str],
    review_id: str,
) -> dict[str, Any]:
    repository = upstream.get("repository")
    tracking_ref = upstream.get("trackingRef")
    discovered_cases = contract.discover_case_ids(snapshot_root)
    case_scopes_by_owner: dict[str, set[str]] = {}
    for case_scope in sorted(executable_case_scopes):
        owner = CASE_SCOPE_OWNERS.get(case_scope)
        if owner is None:
            raise RefreshError(
                f"executable case scope has no explicit owner: {case_scope}"
            )
        case_scopes_by_owner.setdefault(owner, set()).add(case_scope)

    file_records = []
    for path in _filesystem_regular_files(snapshot_root):
        local_path = path.relative_to(snapshot_root).as_posix()
        scope, upstream_path = _full_corpus_file_source(
            local_path,
            affected_skills=affected_skills,
            executable_case_scopes=executable_case_scopes,
        )
        file_records.append(
            {
                "scope": scope,
                "upstreamPath": upstream_path,
                "localPath": local_path,
                "acceptedCommit": target_commit,
                "sha256": contract.sha256_file(path),
            }
        )
    file_records.sort(key=lambda item: item["localPath"])

    scopes = {}
    for scope in sorted(affected_skills):
        scopes[scope] = {
            "ownerSkill": scope,
            "caseScopes": sorted(case_scopes_by_owner.get(scope, set())),
            "acceptedCommit": target_commit,
            "reviewId": review_id,
            "contentDigest": contract.scope_content_digest(
                file_records, scope
            ),
        }

    scripts_by_skill = {skill: [] for skill in sorted(corpus_skill_names)}
    for item in file_records:
        path = item["localPath"]
        parts = Path(path).parts
        if len(parts) >= 4 and parts[0] == "skills" and parts[2] == "scripts":
            scripts_by_skill.setdefault(parts[1], []).append(path)
    corpus_skills = {
        skill: {"scripts": sorted(scripts)}
        for skill, scripts in sorted(scripts_by_skill.items())
    }

    cases_by_scope: dict[str, list[str]] = {}
    cases = {}
    for case_id in discovered_cases:
        case_scope = case_id.split("/", 1)[0]
        cases_by_scope.setdefault(case_scope, []).append(case_id)
        if case_scope in executable_case_scopes:
            owner = CASE_SCOPE_OWNERS[case_scope]
            digest_kind = "execution"
            content_digest = contract.case_content_digest(
                snapshot_root, case_id
            )
        else:
            owner = None
            digest_kind = "corpus"
            content_digest = contract.corpus_case_content_digest(
                snapshot_root, case_id
            )
        cases[case_id] = {
            "scope": owner,
            "digestKind": digest_kind,
            "contentDigest": content_digest,
        }

    shared_files = sorted(
        item["localPath"]
        for item in file_records
        if item["localPath"].startswith("case-runner/")
    )
    suite_files: dict[str, list[str]] = {}
    for item in file_records:
        path = item["localPath"]
        parts = Path(path).parts
        if len(parts) >= 2 and parts[0] == "suites":
            suite_files.setdefault(parts[1], []).append(path)
    corpus_tests = {
        "caseScopes": {
            scope: {"caseIds": sorted(case_ids)}
            for scope, case_ids in sorted(cases_by_scope.items())
        },
        "sharedFiles": shared_files,
        "suites": {
            suite: {"files": sorted(paths)}
            for suite, paths in sorted(suite_files.items())
        },
    }
    return {
        "schemaVersion": 2,
        "upstreamId": UPSTREAM_ID,
        "repository": repository,
        "trackingRef": tracking_ref,
        "acceptedCommit": target_commit,
        "corpusSkills": corpus_skills,
        "corpusTests": corpus_tests,
        "executableCaseScopes": sorted(executable_case_scopes),
        "scopes": scopes,
        "files": file_records,
        "cases": dict(sorted(cases.items())),
    }


def maybe_fetch(repository: Path) -> None:
    probe = subprocess.run(
        ["git", "remote", "get-url", "origin"],
        cwd=repository,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if probe.returncode == 0:
        _git(repository, "fetch", "--prune", "origin")


def resolve_target(repository: Path, target: str) -> str:
    candidates = []
    if not contract.HEX_40.fullmatch(target):
        candidates.append(f"refs/remotes/origin/{target}")
    candidates.append(target)
    for candidate in candidates:
        result = subprocess.run(
            ["git", "rev-parse", "--verify", f"{candidate}^{{commit}}"],
            cwd=repository,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode == 0:
            commit = result.stdout.strip()
            if contract.HEX_40.fullmatch(commit):
                return commit
    raise RefreshError(f"cannot resolve donor target: {target}")


def copy_case_scope(
    repository: Path,
    commit: str,
    scope: str,
    snapshot_root: Path,
) -> list[str]:
    prefix = f"tests/skills/cases/{scope}"
    copied = []
    for mode, object_type, upstream_path in git_tree_entries(
        repository, commit, prefix
    ):
        relative = upstream_path.removeprefix(prefix + "/")
        if not relative or not _case_file_is_selected(relative):
            continue
        _copy_git_blob(
            repository,
            commit,
            mode,
            object_type,
            upstream_path,
            snapshot_root / "cases" / scope / contract.safe_relative_path(relative),
        )
        copied.append(upstream_path)
    if not copied:
        return []
    required = {
        f"{prefix}/_skill.json",
    }
    if not required.issubset(copied):
        raise RefreshError(f"selected case scope is incomplete: {scope}")
    return copied


def copy_skill_scripts(
    repository: Path,
    commit: str,
    skill: str,
    snapshot_root: Path,
) -> list[str]:
    prefix = f".claude/skills/{skill}/scripts"
    copied = []
    for mode, object_type, upstream_path in git_tree_entries(
        repository, commit, prefix
    ):
        relative = upstream_path.removeprefix(
            f".claude/skills/{skill}/"
        )
        destination = (
            snapshot_root / "skills" / skill / contract.safe_relative_path(relative)
        )
        _copy_git_blob(
            repository,
            commit,
            mode,
            object_type,
            upstream_path,
            destination,
        )
        copied.append(upstream_path)
    if not copied:
        raise RefreshError(f"donor scripts are missing for skill: {skill}")
    return copied


def copy_full_corpus(
    repository: Path,
    commit: str,
    snapshot_root: Path,
) -> tuple[set[str], list[str]]:
    donor_skills: set[str] = set()
    copied: list[str] = []
    for mode, object_type, upstream_path in git_tree_entries(
        repository, commit, ".claude/skills"
    ):
        parts = contract.safe_relative_path(upstream_path).parts
        if len(parts) < 4:
            continue
        donor_skills.add(parts[2])
        if len(parts) < 5 or parts[3] != "scripts":
            continue
        relative = Path(*parts[4:])
        destination = (
            snapshot_root
            / "skills"
            / parts[2]
            / "scripts"
            / relative
        )
        _copy_git_blob(
            repository,
            commit,
            mode,
            object_type,
            upstream_path,
            destination,
        )
        copied.append(upstream_path)

    for mode, object_type, upstream_path in git_tree_entries(
        repository, commit, "tests"
    ):
        parts = contract.safe_relative_path(upstream_path).parts
        if len(parts) >= 5 and parts[:3] == ("tests", "skills", "cases"):
            destination = (
                snapshot_root
                / "cases"
                / parts[3]
                / Path(*parts[4:])
            )
        elif len(parts) >= 3 and parts[:2] == ("tests", "skills"):
            destination = snapshot_root / "case-runner" / Path(*parts[2:])
        elif len(parts) >= 2 and parts[0] == "tests":
            destination = snapshot_root / "suites" / Path(*parts[1:])
        else:
            continue
        _copy_git_blob(
            repository,
            commit,
            mode,
            object_type,
            upstream_path,
            destination,
        )
        copied.append(upstream_path)

    if not donor_skills:
        raise RefreshError("donor skill corpus is empty")
    return donor_skills, copied


def git_tree_entries(
    repository: Path, commit: str, prefix: str
) -> list[tuple[str, str, str]]:
    result = subprocess.run(
        [
            "git",
            "ls-tree",
            "-r",
            "-z",
            "--full-tree",
            commit,
            "--",
            prefix,
        ],
        cwd=repository,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    entries = []
    for raw in result.stdout.split(b"\0"):
        if not raw:
            continue
        metadata, raw_path = raw.split(b"\t", 1)
        mode, object_type, _object_id = metadata.decode("ascii").split()
        entries.append(
            (
                mode,
                object_type,
                raw_path.decode("utf-8", errors="strict"),
            )
        )
    return entries


def snapshot_digest(snapshot_root: Path) -> str:
    records = [
        {
            "path": path.relative_to(snapshot_root).as_posix(),
            "sha256": contract.sha256_file(path),
        }
        for path in _filesystem_regular_files(snapshot_root)
    ]
    return contract.sha256_json(records)


def _copy_git_blob(
    repository: Path,
    commit: str,
    mode: str,
    object_type: str,
    upstream_path: str,
    destination: Path,
) -> None:
    if mode == "120000":
        raise RefreshError(f"donor symlink is forbidden: {upstream_path}")
    if object_type != "blob" or mode not in {"100644", "100755"}:
        raise RefreshError(
            f"donor path is not a regular file: {upstream_path} ({mode} {object_type})"
        )
    result = subprocess.run(
        ["git", "show", f"{commit}:{upstream_path}"],
        cwd=repository,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(result.stdout)
    destination.chmod(0o755 if mode == "100755" else 0o644)


def _available_case_scopes(repository: Path, commit: str) -> set[str]:
    prefix = "tests/skills/cases"
    scopes = set()
    for _mode, _object_type, path in git_tree_entries(
        repository, commit, prefix
    ):
        parts = path.split("/")
        if len(parts) >= 4:
            scopes.add(parts[3])
    return scopes


def _case_file_is_selected(relative: str) -> bool:
    path = contract.safe_relative_path(relative)
    if "snapshots" in path.parts:
        return False
    if len(path.parts) == 1:
        return path.suffix == ".json"
    return path.parts[0] == "fixtures"


def _case_dependency_skills(
    snapshot_root: Path, selected_case_scopes: set[str]
) -> set[str]:
    skills = set()
    for scope in selected_case_scopes:
        case_root = snapshot_root / "cases" / scope
        config_path = case_root / "_skill.json"
        if not config_path.is_file():
            continue
        config = contract.load_json(config_path)
        _add_script_skill(skills, config.get("script"))
        post_validate = config.get("postValidate")
        if isinstance(post_validate, dict):
            _add_script_skill(skills, post_validate.get("script"))
        for case_path in sorted(case_root.glob("*.json")):
            if case_path.name.startswith("_"):
                continue
            case = contract.load_json(case_path)
            setup = case.get("setup") or config.get("setup") or "none"
            if setup == "empty-config":
                skills.add("cf-init")
            for step in case.get("preRun") or []:
                if isinstance(step, dict) and "script" in step:
                    _add_script_skill(skills, step.get("script"))
    return skills


def _add_script_skill(skills: set[str], raw: object) -> None:
    if not isinstance(raw, str):
        raise RefreshError(f"invalid donor script dependency: {raw!r}")
    path = contract.safe_relative_path(raw)
    if len(path.parts) != 3 or path.parts[1] != "scripts":
        raise RefreshError(f"invalid donor script dependency: {raw}")
    skills.add(path.parts[0])


def _validate_watched_paths(
    upstream: dict[str, Any], owner: str, paths: Iterable[str]
) -> None:
    entry = next(
        (
            value
            for value in upstream.get("entries") or []
            if isinstance(value, dict) and value.get("skill") == owner
        ),
        None,
    )
    if entry is None:
        raise RefreshError(f"provenance entry is missing for {owner}")
    patterns = entry.get("upstreamPaths") or []
    for path in paths:
        if not any(_matches_watched_path(path, pattern) for pattern in patterns):
            raise RefreshError(
                f"copied upstream path is outside watched scope for {owner}: {path}"
            )


def _matches_watched_path(path: str, pattern: object) -> bool:
    if not isinstance(pattern, str):
        return False
    if pattern.endswith("/**"):
        prefix = pattern.removesuffix("/**")
        return path == prefix or path.startswith(prefix + "/")
    return path == pattern


def _snapshot_file_source(local_path: str) -> tuple[str, str]:
    path = contract.safe_relative_path(local_path)
    if len(path.parts) >= 3 and path.parts[0] == "cases":
        case_scope = path.parts[1]
        owner = CASE_SCOPE_OWNERS.get(case_scope)
        if owner is None:
            raise RefreshError(f"case scope has no explicit owner: {case_scope}")
        return owner, f"tests/skills/{local_path}"
    if len(path.parts) >= 4 and path.parts[0] == "skills":
        donor_skill = path.parts[1]
        return donor_skill_owner(donor_skill), f".claude/{local_path}"
    raise RefreshError(f"unsupported donor snapshot path: {local_path}")


def _full_corpus_file_source(
    local_path: str,
    *,
    affected_skills: set[str],
    executable_case_scopes: set[str],
) -> tuple[str | None, str]:
    path = contract.safe_relative_path(local_path)
    if len(path.parts) >= 3 and path.parts[0] == "cases":
        case_scope = path.parts[1]
        owner = (
            CASE_SCOPE_OWNERS.get(case_scope)
            if case_scope in executable_case_scopes
            else None
        )
        if case_scope in executable_case_scopes and owner is None:
            raise RefreshError(
                f"executable case scope has no explicit owner: {case_scope}"
            )
        return owner, f"tests/skills/{local_path}"
    if len(path.parts) >= 4 and path.parts[0] == "skills":
        owner = donor_skill_owner(path.parts[1])
        return (
            owner if owner in affected_skills else None,
            f".claude/{local_path}",
        )
    if len(path.parts) >= 2 and path.parts[0] == "case-runner":
        relative = Path(*path.parts[1:]).as_posix()
        return None, f"tests/skills/{relative}"
    if len(path.parts) >= 3 and path.parts[0] == "suites":
        relative = Path(*path.parts[1:]).as_posix()
        return None, f"tests/{relative}"
    raise RefreshError(f"unsupported donor snapshot path: {local_path}")


def donor_skill_owner(donor_skill: str) -> str:
    return DONOR_SKILL_OWNERS.get(donor_skill, donor_skill)


def _changed_snapshot_paths(before: Path, after: Path) -> list[str]:
    before_hashes = {
        path.relative_to(before).as_posix(): contract.sha256_file(path)
        for path in _filesystem_regular_files(before)
    }
    after_hashes = {
        path.relative_to(after).as_posix(): contract.sha256_file(path)
        for path in _filesystem_regular_files(after)
    }
    return sorted(
        path
        for path in set(before_hashes) | set(after_hashes)
        if before_hashes.get(path) != after_hashes.get(path)
    )


def _filesystem_regular_files(root: Path) -> list[Path]:
    result = []
    if not root.exists():
        return result
    _reject_filesystem_symlinks(root)
    for current, _directories, files in os.walk(root, followlinks=False):
        current_path = Path(current)
        for name in files:
            path = current_path / name
            if not path.is_file():
                raise RefreshError(f"snapshot path is not a regular file: {path}")
            result.append(path)
    return sorted(result)


def _reject_filesystem_symlinks(root: Path) -> None:
    for current, directories, files in os.walk(root, followlinks=False):
        current_path = Path(current)
        for name in [*directories, *files]:
            path = current_path / name
            if path.is_symlink():
                raise RefreshError(f"snapshot symlink is forbidden: {path}")


def _publish_atomically(
    *,
    review_root: Path,
    accepted_snapshot: Path,
    candidate_snapshot: Path,
    files: dict[Path, dict[str, Any]],
) -> None:
    backup_root = review_root / "backup"
    if backup_root.exists():
        shutil.rmtree(backup_root)
    backup_root.mkdir()
    snapshot_backup = backup_root / "snapshot"
    file_backups: dict[Path, Path | None] = {}
    try:
        if accepted_snapshot.exists():
            shutil.copytree(accepted_snapshot, snapshot_backup)
        for index, target in enumerate(files):
            if target.is_file():
                backup = backup_root / f"file-{index}.json"
                shutil.copy2(target, backup)
                file_backups[target] = backup
            else:
                file_backups[target] = None

        if accepted_snapshot.exists():
            shutil.rmtree(accepted_snapshot)
        accepted_snapshot.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(candidate_snapshot, accepted_snapshot)
        for target, value in files.items():
            _write_json_atomic(target, value)
    except Exception:
        if accepted_snapshot.exists():
            shutil.rmtree(accepted_snapshot)
        if snapshot_backup.exists():
            shutil.copytree(snapshot_backup, accepted_snapshot)
        for target, backup in file_backups.items():
            if backup is None:
                target.unlink(missing_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(backup, target)
        raise
    finally:
        if backup_root.exists():
            shutil.rmtree(backup_root)


def _write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def _write_json_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    _write_json(temporary, value)
    os.replace(temporary, path)


def _git(repository: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", *arguments],
        cwd=repository,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    return result.stdout.strip()


def _find_upstream(provenance: dict[str, Any]) -> dict[str, Any]:
    for upstream in provenance.get("upstreams") or []:
        if isinstance(upstream, dict) and upstream.get("id") == UPSTREAM_ID:
            return upstream
    raise RefreshError(f"provenance upstream is missing: {UPSTREAM_ID}")


def _require_git_repository(path: Path) -> None:
    result = subprocess.run(
        ["git", "rev-parse", "--git-dir"],
        cwd=path,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise RefreshError(f"upstream cache is not a Git repository: {path}")


def _validate_review_id(review_id: object) -> None:
    if not isinstance(review_id, str):
        raise RefreshError("review id must be a string")
    path = contract.safe_relative_path(review_id)
    if len(path.parts) != 1:
        raise RefreshError("review id must be one safe path component")


def _portable_path(repo_root: Path, path: Path) -> str:
    try:
        return path.relative_to(repo_root).as_posix()
    except ValueError:
        return str(path)


def _resolve_portable_path(repo_root: Path, raw: object) -> Path:
    if not isinstance(raw, str) or not raw:
        raise RefreshError("review upstreamCache is missing")
    path = Path(raw)
    return path if path.is_absolute() else repo_root / contract.safe_relative_path(raw)


def _optional_sha256(path: Path) -> str | None:
    return contract.sha256_file(path) if path.is_file() else None


def _verify_digest(path: Path, expected: object, label: str) -> None:
    if not isinstance(expected, str) or not path.is_file():
        raise RefreshError(f"{label} is missing")
    if contract.sha256_file(path) != expected:
        raise RefreshError(f"{label} changed after prepare")


def _verify_optional_digest(
    path: Path, expected: object, label: str
) -> None:
    actual = _optional_sha256(path)
    if actual != expected:
        raise RefreshError(f"{label} changed after prepare")


if __name__ == "__main__":
    raise SystemExit(main())
