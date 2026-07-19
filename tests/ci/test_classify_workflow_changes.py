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


class ClassifyWorkflowChangesTests(unittest.TestCase):
    def test_release_artifacts_are_needed_for_package_surface_changes(self) -> None:
        module = load_classifier_module()

        release_paths = [
            ".agents/plugins/marketplace.json",
            ".github/workflows/unica-plugin-release.yml",
            ".github/workflows/publish-unica-marketplace.yml",
            "Cargo.toml",
            "Cargo.lock",
            "crates/unica-coder/Cargo.toml",
            "crates/unica-bootstrap/Cargo.toml",
            "crates/unica-bootstrap/src/main.rs",
            "crates/unica-coder/build.rs",
            "crates/unica-coder/src/application/mod.rs",
            "crates/unica-coder/src/infrastructure/internal_adapters.rs",
            "plugins/unica/.codex-plugin/plugin.json",
            "plugins/unica/.mcp.json",
            "plugins/unica/third-party/tools.lock.json",
            "plugins/unica/third-party/manifest.json",
            "scripts/ci/check-tool-contracts.py",
            "scripts/ci/release-assessment.py",
            "scripts/ci/build-unica-tools.py",
            "scripts/ci/package-unica-plugin.py",
            "scripts/ci/package-unica-runtime.py",
            "scripts/ci/smoke-unica-bootstrap.py",
            "scripts/ci/verify-release-assets.py",
        ]

        for path in release_paths:
            with self.subTest(path=path):
                self.assertTrue(module.needs_release_artifacts([path]))

    def test_release_artifacts_are_not_needed_for_source_only_pr_changes(self) -> None:
        module = load_classifier_module()

        source_only_paths = [
            "plugins/unica/skills/meta-compile/SKILL.md",
            "plugins/unica/references/tooling/internal-package.md",
            "spec/architecture/invariants.md",
            "tests/ci/test_unica_workflow.py",
            "tests/fixtures/unica_mcp_script_parity/meta-catalog.json",
        ]

        self.assertFalse(module.needs_release_artifacts(source_only_paths))

    def test_cli_prints_boolean_from_stdin_paths(self) -> None:
        module = load_classifier_module()

        with tempfile.TemporaryFile("w+", encoding="utf-8") as stdin:
            stdin.write("tests/ci/test_unica_workflow.py\ncrates/unica-bootstrap/src/main.rs\n")
            stdin.seek(0)

            self.assertEqual(module.classify_stdin(stdin), "true")
