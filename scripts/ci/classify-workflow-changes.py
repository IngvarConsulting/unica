#!/usr/bin/env python3
"""Classify changed paths into typed Unica CI routing contours."""

from __future__ import annotations

import argparse
import sys
from collections.abc import Iterable
from pathlib import PurePosixPath
from typing import NamedTuple, TextIO


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
}
PACKAGE_PATHS = {
    ".agents/plugins/marketplace.json",
    "plugins/unica/.claude-plugin/plugin.json",
    "plugins/unica/.codex-plugin/plugin.json",
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
    "scripts/dev/install-local-unica.sh",
    "scripts/ci/classify-workflow-changes.py",
    "scripts/ci/evaluate-ci-gate.py",
    # Шов прогона решает, что именно гоняет каждая джоба; его правка — правка
    # конвейера, а не исходников, и обязана ехать полным контуром.
    "scripts/ci/run-tests.py",
    "tests/ci/test_run_tests.py",
    "tests/ci/test_classify_workflow_changes.py",
    "tests/ci/test_evaluate_ci_gate.py",
    "tests/ci/test_unica_workflow.py",
}
PLATFORM_CONTRACT_PATHS = {
    "scripts/ci/check-rust-platform-boundary.py",
    "tests/ci/test_rust_platform_boundary.py",
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
    "crates/unica-coder/src/domain/source_revision.rs",
    "crates/unica-coder/src/infrastructure/application_ports.rs",
    "crates/unica-coder/src/infrastructure/code_intelligence.rs",
    "crates/unica-coder/src/infrastructure/internal_adapters.rs",
    "crates/unica-coder/src/infrastructure/operational_config.rs",
    "crates/unica-coder/src/infrastructure/platform/process.rs",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
    "crates/unica-coder/src/infrastructure/rlm_navigation.rs",
    "crates/unica-coder/src/infrastructure/source_revision.rs",
    "crates/unica-coder/src/infrastructure/source_roots.rs",
    "crates/unica-coder/src/infrastructure/workspace_index.rs",
    "crates/unica-coder/src/infrastructure/workspace_services.rs",
    "crates/unica-coder/src/interfaces/mcp.rs",
    "crates/unica-coder/tests/issue_89_workspace_service.rs",
    "crates/unica-coder/tests/platform/issue_89_workspace_service.rs",
}
SEARCH_INTEGRATION_PREFIXES = (
    "crates/unica-coder/src/infrastructure/platform/source_revision_fence",
)
# The long BSP assessment runs the packaged runtime end to end, so it follows
# the search contour it exercises. These paths add what that contour does not
# name: the runtime the assessment unpacks, and the routing that decides
# whether the assessment runs at all.
ASSESSMENT_PATHS = {
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
    return len(candidate.parts) >= 2 and candidate.parts[0] == "crates" and candidate.suffix == ".rs"


def _is_toolchain_path(path: str) -> bool:
    candidate = PurePosixPath(path)
    return (
        path in TOOLCHAIN_PATHS
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
            or path.startswith(SEARCH_INTEGRATION_PREFIXES)
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--force-full",
        action="store_true",
        help="enable every contour for tags, manual runs, or the ci:full label",
    )
    args = parser.parse_args()
    print(classify_stdin(sys.stdin, force_full=args.force_full))


if __name__ == "__main__":
    main()
