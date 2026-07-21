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
    "package_changed",
    "plugin_content_changed",
    "ci_changed",
    "release_required",
)


class ClassifyWorkflowChangesTests(unittest.TestCase):
    def assert_classification(self, paths: list[str], **expected: bool) -> None:
        module = load_classifier_module()
        classification = module.classify_paths(paths)
        actual = classification.as_dict()
        self.assertEqual(set(OUTPUT_NAMES), set(actual))
        self.assertEqual({name: expected.get(name, False) for name in OUTPUT_NAMES}, actual)

    def test_skill_or_provenance_only_change_stays_in_source_contour(self) -> None:
        self.assert_classification(
            [
                "plugins/unica/skills/meta-compile/SKILL.md",
                "plugins/unica/provenance/skill-upstreams.json",
            ],
            plugin_content_changed=True,
        )

    def test_platform_independent_domain_or_application_rust_uses_primary_rust_contour(self) -> None:
        for path in (
            "crates/unica-coder/src/domain/cache.rs",
            "crates/unica-coder/src/application/ports.rs",
        ):
            with self.subTest(path=path):
                self.assert_classification([path], rust_changed=True, release_required=True)

    def test_platform_facade_and_platform_tests_require_platform_matrix(self) -> None:
        for path in (
            "crates/unica-coder/src/infrastructure/platform/filesystem.rs",
            "crates/unica-bootstrap/src/platform/mod.rs",
            "crates/unica-coder/tests/platform/new_contract.rs",
            "crates/unica-coder/tests/platform_external_init.rs",
            "crates/unica-coder/src/infrastructure/platform/unknown.future",
        ):
            with self.subTest(path=path):
                self.assert_classification(
                    [path],
                    rust_changed=True,
                    platform_changed=True,
                    release_required=True,
                )

    def test_cargo_and_toolchain_changes_require_full_rust_and_package_contours(self) -> None:
        for path in ("Cargo.toml", "Cargo.lock", "crates/unica-coder/Cargo.toml", "rust-toolchain.toml"):
            with self.subTest(path=path):
                self.assert_classification(
                    [path],
                    rust_changed=True,
                    toolchain_changed=True,
                    package_changed=True,
                    release_required=True,
                )

    def test_package_contract_changes_require_package_contour(self) -> None:
        for path in (
            "plugins/unica/.mcp.json",
            "plugins/unica/third-party/tools.lock.json",
            "scripts/ci/package-unica-runtime.py",
        ):
            with self.subTest(path=path):
                self.assert_classification(
                    [path],
                    package_changed=True,
                    plugin_content_changed=path.startswith("plugins/unica/"),
                    release_required=True,
                )

    def test_classifier_workflow_and_platform_guard_changes_fail_closed(self) -> None:
        cases = {
            ".github/workflows/unica-plugin-release.yml": {"ci_changed": True},
            "scripts/ci/classify-workflow-changes.py": {"ci_changed": True},
            "tests/ci/test_classify_workflow_changes.py": {"ci_changed": True},
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

    def test_local_installer_change_requires_ci_contour(self) -> None:
        self.assert_classification(
            ["scripts/dev/install-local-unica.sh"],
            ci_changed=True,
        )

    def test_mixed_changes_union_their_contours(self) -> None:
        self.assert_classification(
            [
                "plugins/unica/skills/meta-compile/SKILL.md",
                "crates/unica-coder/src/infrastructure/platform/process.rs",
                "plugins/unica/.codex-plugin/plugin.json",
            ],
            rust_changed=True,
            platform_changed=True,
            package_changed=True,
            plugin_content_changed=True,
            release_required=True,
        )

    def test_forced_full_contour_enables_every_output(self) -> None:
        module = load_classifier_module()

        classification = module.classify_paths([], force_full=True)

        self.assertEqual({name: True for name in OUTPUT_NAMES}, classification.as_dict())

    def test_cli_prints_github_outputs_from_stdin_paths(self) -> None:
        module = load_classifier_module()
        with tempfile.TemporaryFile("w+", encoding="utf-8") as stdin:
            stdin.write("plugins/unica/skills/meta-compile/SKILL.md\ncrates/unica-bootstrap/src/main.rs\n")
            stdin.seek(0)

            output = module.classify_stdin(stdin)

        self.assertEqual(
            {
                "rust_changed=true",
                "platform_changed=false",
                "toolchain_changed=false",
                "package_changed=false",
                "plugin_content_changed=true",
                "ci_changed=false",
                "release_required=true",
            },
            set(output.splitlines()),
        )


if __name__ == "__main__":
    unittest.main()
