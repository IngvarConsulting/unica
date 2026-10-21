#!/usr/bin/env python3
"""Classify changed paths into typed Unica CI routing contours."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from collections.abc import Iterable
from pathlib import Path, PurePosixPath
from typing import NamedTuple, TextIO

sys.path.insert(0, str(Path(__file__).resolve().parent))
import allure_results  # noqa: E402
from workflow_routing import rust_route  # noqa: E402


OUTPUT_NAMES = (
    "rust_changed",
    "platform_changed",
    "toolchain_changed",
    "search_integration_changed",
    "package_changed",
    "plugin_content_changed",
    "ci_changed",
    "release_required",
    "assessment_required",
)

TOOLCHAIN_PATHS = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain",
    "rust-toolchain.toml",
    # Настройка исполнителя тестов меняет, как гоняется каждая цель на каждом
    # раннере: это тот же класс, что смена toolchain.
    ".config/nextest.toml",
    ".config/python-sizes.toml",
}
PACKAGE_PATHS = {
    ".agents/plugins/marketplace.json",
    "plugins/unica/.claude-plugin/plugin.json",
    "plugins/unica/.codex-plugin/plugin.json",
    "plugins/unica/.zcode-plugin/plugin.json",
    "plugins/unica/.mcp.json",
    "plugins/unica/bootstrap/launch.sh",
    "plugins/unica/runtime-manifest.json",
    "plugins/unica/third-party/tools.lock.json",
    "plugins/unica/third-party/manifest.json",
    "scripts/ci/build-unica-tools.py",
    "scripts/ci/check-tool-contracts.py",
    "scripts/ci/package-unica-plugin.py",
    "scripts/ci/package-unica-runtime.py",
    "scripts/ci/probe-unica-wire.py",
    "scripts/ci/release-assessment.py",
    "scripts/ci/release-proof.py",
    "scripts/ci/stage-unica-assessment-engine.py",
    "tests/ci/test_stage_unica_assessment_engine.py",
    # The test travels with the script it guards. Without it a test-only change
    # would claim the assessment contour while claiming no release or CI
    # contour, and `evaluate-ci-gate.py` rejects that pair as contradictory.
    "tests/ci/test_release_assessment.py",
    "tests/ci/test_release_proof.py",
    "scripts/ci/smoke-unica-bootstrap.py",
    "scripts/ci/smoke-unica-mcp.py",
    "scripts/ci/verify-release-assets.py",
}
CI_CONTRACT_PATHS = {
    "scripts/ci/check-component-boundaries.py",
    "tests/ci/test_component_boundaries.py",
    "scripts/ci/workflow_routing.py",
    "scripts/dev/install-local-unica.sh",
    "scripts/ci/classify-workflow-changes.py",
    "scripts/ci/evaluate-ci-gate.py",
    # Шов прогона решает, что именно гоняет каждая джоба; его правка — правка
    # конвейера, а не исходников, и обязана ехать полным контуром.
    "scripts/ci/run-tests.py",
    "scripts/ci/ci_rust_lanes.py",
    "scripts/ci/prepared_binary.py",
    "scripts/ci/ci_shadow_selector.py",
    "scripts/ci/rust_inventory_baseline.py",
    "tests/ci/test_ci_rust_lanes.py",
    "tests/ci/test_rust_inventory_baseline.py",
    "tests/ci/test_ci_prepared_binary.py",
    "tests/ci/test_ci_shadow_selector.py",
    "scripts/ci/run-unittest.py",
    "scripts/ci/allure_results.py",
    "scripts/ci/collect-results.py",
    "tests/ci/test_run_tests.py",
    "tests/ci/test_gate_profiles.py",
    "scripts/ci/nightly-lines.py",
    "scripts/ci/resolve-line.py",
    "tests/ci/test_nightly_lines.py",
    "tests/ci/test_resolve_line.py",
    "scripts/ci/size-filters.py",
    "tests/ci/test_size_guard.py",
    "tests/ci/test_python_sizes.py",
    "tests/ci/test_research_policy.py",
    "tests/ci/test_allure_results.py",
    "tests/ci/test_collect_results.py",
    "tests/ci/test_scope_skips.py",
    "tests/ci/test_classify_workflow_changes.py",
    "tests/ci/test_evaluate_ci_gate.py",
    "tests/ci/test_unica_workflow.py",
}
PLATFORM_CONTRACT_PATHS = {
    "scripts/ci/check-rust-platform-boundary.py",
    "tests/ci/test_rust_platform_boundary.py",
    # Проход Clippy по файловым методам (#1894): конфигурация, проба и скрипт.
    "scripts/ci/check-file-io-methods.py",
    "tests/ci/test_file_io_methods.py",
    ".config/clippy/file-io-free/clippy.toml",
    "scripts/ci/fixtures/file-io-probe/Cargo.toml",
    "scripts/ci/fixtures/file-io-probe/Cargo.lock",
    "scripts/ci/fixtures/file-io-probe/src/lib.rs",
}
PLATFORM_SOURCE_PATHS = {
    "crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs",
    "crates/unica-coder/src/infrastructure/path_policy.rs",
    "crates/unica-coder/src/infrastructure/platform_xml_owner.rs",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
    "crates/unica-coder/src/infrastructure/source_roots.rs",
    "crates/unica-coder/src/infrastructure/support_state.rs",
}
PLATFORM_PREFIXES = (
    "crates/unica-coder/src/infrastructure/platform/",
    "crates/unica-bootstrap/src/platform/",
)
SEARCH_INTEGRATION_OWNER_PREFIXES = (
    "crates/unica-code-intelligence/",
    "crates/unica-configuration/",
    "crates/unica-source-contract/",
)
SEARCH_INTEGRATION_PATHS = {
    "crates/unica-coder/src/application/code_intelligence.rs",
    "crates/unica-coder/src/application/mod.rs",
    "crates/unica-coder/src/application/operational_config.rs",
    "crates/unica-coder/src/application/ports.rs",
    "crates/unica-coder/src/application/source_navigation.rs",
    "crates/unica-coder/src/application/tool_contracts.rs",
    "crates/unica-coder/src/domain/code_intelligence.rs",
    "crates/unica-coder/src/domain/operational_config.rs",
    "crates/unica-coder/src/domain/source_location.rs",
    "crates/unica-coder/src/infrastructure/application_ports.rs",
    "crates/unica-coder/src/infrastructure/code_intelligence.rs",
    "crates/unica-coder/src/infrastructure/internal_adapters.rs",
    "crates/unica-coder/src/infrastructure/operational_config.rs",
    "crates/unica-coder/src/infrastructure/platform/process.rs",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
    "crates/unica-coder/src/infrastructure/rlm_navigation.rs",
    "crates/unica-coder/src/infrastructure/source_roots.rs",
    "crates/unica-coder/src/infrastructure/workspace_index.rs",
    "crates/unica-coder/src/infrastructure/workspace_services.rs",
    "crates/unica-coder/src/interfaces/mcp.rs",
}
# The long BSP assessment runs the packaged runtime end to end, so it follows
# the search contour it exercises. These paths add what that contour does not
# name: the runtime the assessment unpacks, and the routing that decides
# whether the assessment runs at all.
ASSESSMENT_PATHS = {
    "scripts/ci/workflow_routing.py",
    ".github/workflows/unica-plugin-release.yml",
    "crates/unica-coder/src/infrastructure/plugin_runtime.rs",
    "plugins/unica/third-party/tools.lock.json",
    "scripts/ci/build-unica-tools.py",
    "scripts/ci/classify-workflow-changes.py",
    "scripts/ci/evaluate-ci-gate.py",
    "scripts/ci/package-unica-plugin.py",
    "scripts/ci/package-unica-runtime.py",
    "scripts/ci/probe-unica-wire.py",
    "scripts/ci/release-assessment.py",
    "scripts/ci/release-proof.py",
    "scripts/ci/stage-unica-assessment-engine.py",
    "scripts/ci/verify-release-assets.py",
    "tests/ci/test_stage_unica_assessment_engine.py",
    "tests/ci/test_classify_workflow_changes.py",
    "tests/ci/test_evaluate_ci_gate.py",
    "tests/ci/test_release_assessment.py",
    "tests/ci/test_release_proof.py",
    "tests/ci/test_unica_workflow.py",
}


class Classification(NamedTuple):
    rust_changed: bool = False
    platform_changed: bool = False
    toolchain_changed: bool = False
    search_integration_changed: bool = False
    package_changed: bool = False
    plugin_content_changed: bool = False
    ci_changed: bool = False
    release_required: bool = False
    assessment_required: bool = False

    def as_dict(self) -> dict[str, bool]:
        return dict(zip(OUTPUT_NAMES, self, strict=True))

    def as_github_output(self) -> str:
        return "\n".join(
            f"{name}={str(value).lower()}" for name, value in self.as_dict().items()
        )


def normalize_path(path: str) -> str:
    normalized = path.strip().replace("\\", "/")
    while normalized.startswith("./"):
        normalized = normalized[2:]
    return normalized


def _is_rust_path(path: str) -> bool:
    candidate = PurePosixPath(path)
    return (
        len(candidate.parts) >= 2
        and (candidate.parts[0] == "crates" or path.startswith("vendor/ureq/"))
        and candidate.suffix == ".rs"
    )


def _is_toolchain_path(path: str) -> bool:
    candidate = PurePosixPath(path)
    return (
        path in TOOLCHAIN_PATHS
        or path in {"vendor/ureq/Cargo.toml", "vendor/ureq/Cargo.lock", "vendor/ureq/.config/nextest.toml"}
        or path.startswith(".cargo/")
        or (len(candidate.parts) >= 3 and candidate.parts[0] == "crates" and candidate.name == "Cargo.toml")
    )


def _is_platform_path(path: str) -> bool:
    candidate = PurePosixPath(path)
    platform_test_directory = (
        len(candidate.parts) >= 5
        and candidate.parts[0] == "crates"
        and candidate.parts[2] == "tests"
        and candidate.parts[3] == "platform"
    )
    platform_test_wrapper = (
        len(candidate.parts) == 4
        and candidate.parts[0] == "crates"
        and candidate.parts[2] == "tests"
        and candidate.name.startswith("platform_")
    )
    return (
        path in PLATFORM_CONTRACT_PATHS
        or path in PLATFORM_SOURCE_PATHS
        or path.startswith(PLATFORM_PREFIXES)
        or platform_test_directory
        or platform_test_wrapper
    )


def _is_ci_contract_path(path: str) -> bool:
    return path.startswith(".github/workflows/") or path in CI_CONTRACT_PATHS or path in PLATFORM_CONTRACT_PATHS


def _is_assessment_path(path: str) -> bool:
    return path in ASSESSMENT_PATHS


def classify_paths(paths: Iterable[str], *, force_full: bool = False) -> Classification:
    if force_full:
        return Classification(*([True] * len(OUTPUT_NAMES)))

    rust_changed = False
    platform_changed = False
    toolchain_changed = False
    search_integration_changed = False
    package_changed = False
    plugin_content_changed = False
    ci_changed = False
    assessment_required = False

    for raw_path in paths:
        path = normalize_path(raw_path)
        if not path:
            continue
        rust_changed |= _is_rust_path(path)
        platform_changed |= _is_platform_path(path)
        toolchain_changed |= _is_toolchain_path(path)
        search_integration_changed |= (
            path in SEARCH_INTEGRATION_PATHS
            or (_is_rust_path(path) and path.startswith(SEARCH_INTEGRATION_OWNER_PREFIXES))
            or _is_toolchain_path(path)
        )
        package_changed |= path in PACKAGE_PATHS or _is_toolchain_path(path)
        plugin_content_changed |= path.startswith("plugins/unica/")
        ci_changed |= _is_ci_contract_path(path)
        assessment_required |= _is_assessment_path(path)

    # The platform boundary is the source of truth. Unknown files inside it
    # must route conservatively even when their extension is not yet known.
    rust_changed |= platform_changed or toolchain_changed
    # The assessment drives the search mechanism against a real BSP, so every
    # path that routes the search contour routes the assessment too. Keeping it
    # a derivation rather than a second list is what stops the two from
    # drifting apart.
    assessment_required |= search_integration_changed
    release_required = rust_changed or package_changed
    return Classification(
        rust_changed=rust_changed,
        platform_changed=platform_changed,
        toolchain_changed=toolchain_changed,
        search_integration_changed=search_integration_changed,
        package_changed=package_changed,
        plugin_content_changed=plugin_content_changed,
        ci_changed=ci_changed,
        release_required=release_required,
        assessment_required=assessment_required,
    )


def classify_stdin(stdin: TextIO, *, force_full: bool = False) -> str:
    return classify_paths(stdin, force_full=force_full).as_github_output()


# These files are editorial inputs. Plugin instructions/specifications, generated
# tool ledgers, provenance data, fixtures and website code deliberately stay out.
PROSE_PATHS = frozenset({
    "README.md", "CONTRIBUTING.md", "CODE_OF_CONDUCT.md", "AGENTS.md", "AI_DEV.md",
    "docs/acceptance-scenarios.md", "docs/internal-package.md", "docs/release-runbook.md",
    "docs/visual-kit/README.md",
})


def is_prose_path(path: str) -> bool:
    candidate = PurePosixPath(path)
    if (str(candidate) != path or candidate.is_absolute() or ".." in candidate.parts
            or "\\" in path or any(ord(char) < 32 for char in path)):
        return False
    return path in PROSE_PATHS or (path.startswith("arch/") and candidate.suffix == ".md")


class ChangedFile(NamedTuple):
    path: str
    status: str
    old_mode: str
    new_mode: str


def parse_raw_diff(raw: str) -> list[ChangedFile]:
    """Read --raw -z --no-renames without quoting/newline ambiguity."""
    if not raw:
        return []
    fields = raw.split("\0")
    if fields.pop() != "" or len(fields) % 2:
        raise ValueError("incomplete raw diff")
    changes = []
    for index in range(0, len(fields), 2):
        header = fields[index].split()
        if len(header) != 5 or not header[0].startswith(":") or header[4] not in {"A", "M", "D", "T"}:
            raise ValueError("unsupported raw diff record")
        changes.append(ChangedFile(fields[index + 1], header[4], header[0][1:], header[1]))
    return changes


def prose_only(changes: list[ChangedFile]) -> bool:
    return bool(changes) and all(
        is_prose_path(change.path)
        and change.new_mode == "100644"
        and ((change.status == "M" and change.old_mode == "100644")
             or (change.status == "A" and change.old_mode == "000000"))
        for change in changes
    )


class EventScope(NamedTuple):
    classification: Classification
    docs_only: bool
    base_sha: str
    head_sha: str


def classify_event(repo: Path, event_name: str, ref: str, sha: str, event: dict) -> EventScope:
    """Compare the checked-out tree against immutable event bounds; unknown is full."""
    full = EventScope(classify_paths((), force_full=True), False, "", sha)

    def git(*args: str) -> str:
        return subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True).stdout

    def commit(value: object) -> str:
        if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{40}", value) is None:
            raise ValueError("missing immutable commit")
        git("cat-file", "-e", f"{value}^{{commit}}")
        return value

    try:
        head = commit(sha)
        if git("rev-parse", "HEAD").strip() != head:
            return full
        if event_name == "pull_request":
            pr = event["pull_request"]
            if any(label.get("name") == "ci:full" for label in pr.get("labels", [])):
                return full
            base, incoming = commit(pr["base"]["sha"]), commit(pr["head"]["sha"])
            # Checkout must be the actual merge of the event base and PR head.
            # A moving ref, stale payload or conflict resolution outside that
            # tree cannot earn the documentation exception.
            if git("rev-list", "--parents", "-n", "1", head).split()[1:] != [base, incoming]:
                return full
        elif event_name == "merge_group":
            group = event["merge_group"]
            base = commit(group["base_sha"])
            if commit(group["head_sha"]) != head:
                return full
            git("merge-base", "--is-ancestor", base, head)
        elif event_name == "push" and ref == "refs/heads/main":
            base = commit(event["before"])
            if commit(event["after"]) != head:
                return full
        else:
            return full
        changes = parse_raw_diff(git("diff", "--raw", "-z", "--no-renames", base, head, "--"))
        if not changes:
            return full
        docs = event_name in {"pull_request", "merge_group"} and prose_only(changes)
        # A code-bearing group always keeps the complete queue contour.
        # Deletions, renames (D+A) and type/mode changes also bypass the PR's
        # existing path-only routing, which does not describe these changes.
        uncertain = any(
            change.status not in {"A", "M"}
            or change.new_mode != "100644"
            or (change.status == "M" and change.old_mode != change.new_mode)
            for change in changes
        )
        force = (event_name == "merge_group" and not docs) or uncertain
        return EventScope(classify_paths((change.path for change in changes), force_full=force), docs, base, head)
    except (KeyError, TypeError, ValueError, AttributeError, subprocess.CalledProcessError, UnicodeError):
        return full


def write_scope_skip(scope: EventScope, out: Path, runners: list[str], *, profile: str, line: str) -> None:
    """Record a routing decision, never a plan or a successful test execution."""
    operating_systems = {"ubuntu-latest": "Linux", "macos-14": "macOS"}
    if profile not in {"pr", "queue"} or not line or not scope.docs_only:
        raise ValueError("a documentation skip requires a PR/queue profile and a line")
    if not runners or len(set(runners)) != len(runners) or any(runner not in operating_systems for runner in runners):
        raise ValueError("unexpected runners for the documentation exception")
    for runner in runners:
        path = allure_results.write_run(out / f"results-rust-{runner}", profile=profile,
            runner=runner, ecosystem="rust", line=line, sha=scope.head_sha)
        marker = json.loads(path.read_text(encoding="utf-8"))
        marker["os"] = operating_systems[runner]
        marker["scope_skip"] = {"reason": "docs_only", "base_sha": scope.base_sha, "head_sha": scope.head_sha}
        path.write_text(json.dumps(marker, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--force-full",
        action="store_true",
        help="enable every contour for tags, manual runs, or the ci:full label",
    )
    parser.add_argument("--github-event", action="store_true", help="route the checked-out GitHub event using immutable commit bounds")
    parser.add_argument("--runner-matrix", help="JSON runners for documentation skip metadata")
    parser.add_argument("--line", help="report line resolved by the workflow")
    parser.add_argument("--results", type=Path, help="where to record documentation skip metadata")
    args = parser.parse_args()
    if not args.github_event:
        print(classify_stdin(sys.stdin, force_full=args.force_full))
        return
    event_name, ref, sha = (os.environ.get(name, "") for name in ("GITHUB_EVENT_NAME", "GITHUB_REF", "GITHUB_SHA"))
    try:
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
    except (KeyError, OSError, ValueError):
        event = {}
    scope = classify_event(Path.cwd(), event_name, ref, sha, event)
    if args.force_full:
        scope = EventScope(classify_paths((), force_full=True), False, "", sha)
    route = rust_route(event_name, ref, scope.classification.as_dict(), docs_only=scope.docs_only)
    if scope.docs_only:
        if args.results is None or not args.runner_matrix:
            parser.error("--results and --runner-matrix are required for a documentation skip")
        runners = json.loads(args.runner_matrix)
        if not isinstance(runners, list) or not all(isinstance(runner, str) for runner in runners):
            parser.error("--runner-matrix must be a JSON list of runner names")
        write_scope_skip(scope, args.results, runners, profile=os.environ.get("GATE_PROFILE", ""), line=args.line or "")
    print(scope.classification.as_github_output())
    for name, value in (("docs_only", scope.docs_only), ("rust_required", route.required), ("cache_required", route.warm_cache)):
        print(f"{name}={str(value).lower()}")


if __name__ == "__main__":
    main()
