from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


def load_classifier_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "classify-workflow-changes.py"
    spec = importlib.util.spec_from_file_location("classify_workflow_changes", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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


class ClassifyWorkflowChangesTests(unittest.TestCase):
    def assert_classification(self, paths: list[str], **expected: bool) -> None:
        module = load_classifier_module()
        classification = module.classify_paths(paths)
        actual = classification.as_dict()
        self.assertEqual(set(OUTPUT_NAMES), set(actual))
        self.assertEqual({name: expected.get(name, False) for name in OUTPUT_NAMES}, actual)

    def test_skill_only_change_stays_in_source_contour(self) -> None:
        self.assert_classification(
            ["plugins/unica/skills/meta-add/SKILL.md"],
            plugin_content_changed=True,
        )

    def test_maintainer_provenance_is_not_plugin_content(self) -> None:
        """The donor index and its review records ship with nothing.

        They live outside `plugins/unica/`, so a change to them must not claim
        the plugin content contour. `verify-source` runs unconditionally and
        still covers the attribution and provenance contracts.
        """
        self.assert_classification(
            [
                "docs/provenance/skill-upstreams.json",
                "docs/provenance/reviews/2026-06-15-upstream-review.json",
            ],
        )

    def test_platform_independent_domain_or_application_rust_uses_primary_rust_contour(self) -> None:
        for path in (
            "crates/unica-coder/src/domain/cache.rs",
            "crates/unica-coder/src/application/metadata.rs",
        ):
            with self.subTest(path=path):
                self.assert_classification([path], rust_changed=True, release_required=True)

    def test_platform_facade_and_platform_tests_require_platform_matrix(self) -> None:
        for path in (
            "crates/unica-coder/src/infrastructure/platform/filesystem.rs",
            "crates/unica-coder/src/infrastructure/platform_xml_owner.rs",
            "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
            "crates/unica-coder/src/infrastructure/source_roots.rs",
            "crates/unica-coder/src/infrastructure/path_policy.rs",
            "crates/unica-coder/src/infrastructure/support_state.rs",
            "crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs",
            "crates/unica-bootstrap/src/platform/mod.rs",
            "crates/unica-coder/tests/platform/new_contract.rs",
            "crates/unica-coder/tests/platform_external_init.rs",
            "crates/unica-coder/src/infrastructure/platform/unknown.future",
        ):
            with self.subTest(path=path):
                search_integration_changed = path in {
                    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
                    "crates/unica-coder/src/infrastructure/source_roots.rs",
                }
                self.assert_classification(
                    [path],
                    rust_changed=True,
                    platform_changed=True,
                    search_integration_changed=search_integration_changed,
                    release_required=True,
                    assessment_required=search_integration_changed,
                )

    def test_cargo_and_toolchain_changes_require_full_rust_and_package_contours(self) -> None:
        for path in ("Cargo.toml", "Cargo.lock", "crates/unica-coder/Cargo.toml", "rust-toolchain.toml"):
            with self.subTest(path=path):
                self.assert_classification(
                    [path],
                    rust_changed=True,
                    toolchain_changed=True,
                    search_integration_changed=True,
                    package_changed=True,
                    release_required=True,
                    assessment_required=True,
                )

    def test_search_and_rlm_mechanism_changes_route_the_long_integration_test(self) -> None:
        for path in (
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
            "crates/unica-coder/src/infrastructure/platform/source_revision_fence.rs",
            "crates/unica-coder/tests/issue_89_workspace_service.rs",
            "crates/unica-coder/tests/platform/issue_89_workspace_service.rs",
        ):
            with self.subTest(path=path):
                platform_changed = (
                    "/platform/" in path
                    or "/tests/platform/" in path
                    or path
                    in {
                        "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
                        "crates/unica-coder/src/infrastructure/source_roots.rs",
                    }
                )
                self.assert_classification(
                    [path],
                    rust_changed=True,
                    platform_changed=platform_changed,
                    search_integration_changed=True,
                    release_required=True,
                    assessment_required=True,
                )

    def test_package_contract_changes_require_package_contour(self) -> None:
        for path in (
            "plugins/unica/.mcp.json",
            "plugins/unica/third-party/tools.lock.json",
            "scripts/ci/package-unica-runtime.py",
            "scripts/ci/stage-unica-assessment-engine.py",
        ):
            with self.subTest(path=path):
                self.assert_classification(
                    [path],
                    package_changed=True,
                    plugin_content_changed=path.startswith("plugins/unica/"),
                    release_required=True,
                    assessment_required=path in {
                        "plugins/unica/third-party/tools.lock.json",
                        "scripts/ci/package-unica-runtime.py",
                        "scripts/ci/stage-unica-assessment-engine.py",
                    },
                )

    def test_classifier_workflow_and_platform_guard_changes_fail_closed(self) -> None:
        cases = {
            ".github/workflows/unica-plugin-release.yml": {
                "ci_changed": True,
                "assessment_required": True,
            },
            "scripts/ci/classify-workflow-changes.py": {
                "ci_changed": True,
                "assessment_required": True,
            },
            "tests/ci/test_classify_workflow_changes.py": {
                "ci_changed": True,
                "assessment_required": True,
            },
            "scripts/ci/check-rust-platform-boundary.py": {
                "rust_changed": True,
                "platform_changed": True,
                "ci_changed": True,
                "release_required": True,
            },
            "tests/ci/test_rust_platform_boundary.py": {
                "rust_changed": True,
                "platform_changed": True,
                "ci_changed": True,
                "release_required": True,
            },
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                self.assert_classification([path], **expected)

    def test_release_matrix_reports_standalone_runtime_size_evidence(self) -> None:
        workflow = (
            Path(__file__).resolve().parents[2]
            / ".github"
            / "workflows"
            / "unica-plugin-release.yml"
        ).read_text(encoding="utf-8")

        self.assertIn("Report standalone runtime size evidence", workflow)
        for label in (
            "RLM source archive bytes",
            "RLM extracted payload bytes",
            "Final runtime archive bytes",
            "Runtime file count",
        ):
            with self.subTest(label=label):
                self.assertIn(label, workflow)
        self.assertIn('manifest["runtimeFiles"]', workflow)
        self.assertIn('archive.extractfile("manifest.json")', workflow)

    def test_local_installer_change_requires_ci_contour(self) -> None:
        self.assert_classification(
            ["scripts/dev/install-local-unica.sh"],
            ci_changed=True,
        )

    def test_p0_release_proof_changes_route_package_and_assessment_contours(self) -> None:
        self.assert_classification(
            ["scripts/ci/release-proof.py"],
            package_changed=True,
            release_required=True,
            assessment_required=True,
        )

    def test_mixed_changes_union_their_contours(self) -> None:
        self.assert_classification(
            [
                "plugins/unica/skills/meta-add/SKILL.md",
                "crates/unica-coder/src/infrastructure/platform/process.rs",
                "plugins/unica/.codex-plugin/plugin.json",
            ],
            rust_changed=True,
            platform_changed=True,
            search_integration_changed=True,
            package_changed=True,
            plugin_content_changed=True,
            release_required=True,
            assessment_required=True,
        )

    def test_release_assessment_routes_only_affected_mechanism(self) -> None:
        """The hour-long BSP assessment is not the price of any Rust change.

        The contour above proves the search mechanism routes it. What this one
        states is the boundary: unrelated Rust stays out, and the packaged
        runtime the assessment unpacks routes it without belonging to the
        search contour at all.
        """
        self.assert_classification(
            ["crates/unica-coder/src/domain/cache.rs"],
            rust_changed=True,
            release_required=True,
        )

        self.assert_classification(
            ["crates/unica-coder/src/infrastructure/plugin_runtime.rs"],
            rust_changed=True,
            release_required=True,
            assessment_required=True,
        )

        self.assert_classification(
            ["scripts/ci/release-assessment.py"],
            package_changed=True,
            release_required=True,
            assessment_required=True,
        )

    def test_every_assessment_path_also_claims_a_release_or_ci_contour(self) -> None:
        """`evaluate-ci-gate.py` reads a lone assessment contour as a contradiction.

        The assessment job hangs off the package pipeline, so a path that
        routes it while claiming neither contour describes a run that cannot
        happen — and the gate fails the whole workflow rather than the one
        job. Deriving the contour from the search paths hides this, so the
        explicit list is what needs stating.
        """
        module = load_classifier_module()

        offenders = []
        for path in sorted(module.ASSESSMENT_PATHS):
            values = module.classify_paths([path]).as_dict()
            if not (values["release_required"] or values["ci_changed"]):
                offenders.append(path)

        self.assertEqual([], offenders)

    def test_release_assessment_affected_contour_is_closed(self) -> None:
        module = load_classifier_module()

        for path in sorted(module.ASSESSMENT_PATHS):
            with self.subTest(affected=path):
                self.assertTrue(module.classify_paths([path]).assessment_required)
        for path in (
            "README.md",
            "plugins/unica/skills/meta-add/SKILL.md",
            "crates/unica-coder/src/domain/cache.rs",
            "docs/provenance/skill-upstreams.json",
        ):
            with self.subTest(unaffected=path):
                self.assertFalse(module.classify_paths([path]).assessment_required)
        self.assertTrue(module.classify_paths([], force_full=True).assessment_required)

    def test_forced_full_contour_enables_every_output(self) -> None:
        module = load_classifier_module()

        classification = module.classify_paths([], force_full=True)

        self.assertEqual({name: True for name in OUTPUT_NAMES}, classification.as_dict())

    def test_cli_prints_github_outputs_from_stdin_paths(self) -> None:
        module = load_classifier_module()
        with tempfile.TemporaryFile("w+", encoding="utf-8") as stdin:
            stdin.write("plugins/unica/skills/meta-add/SKILL.md\ncrates/unica-bootstrap/src/main.rs\n")
            stdin.seek(0)

            output = module.classify_stdin(stdin)

        self.assertEqual(
            {
                "rust_changed=true",
                "platform_changed=false",
                "toolchain_changed=false",
                "search_integration_changed=false",
                "package_changed=false",
                "plugin_content_changed=true",
                "ci_changed=false",
                "release_required=true",
                "assessment_required=false",
            },
            set(output.splitlines()),
        )


if __name__ == "__main__":
    unittest.main()
