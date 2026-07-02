from __future__ import annotations

import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-plugin-release.yml"


class UnicaWorkflowGuardrailTests(unittest.TestCase):
    def workflow_text(self) -> str:
        return WORKFLOW.read_text(encoding="utf-8")

    def test_pull_request_paths_cover_all_plugin_sources(self) -> None:
        text = self.workflow_text()
        required_paths = [
            ".agents/plugins/marketplace.json",
            ".github/workflows/unica-plugin-release.yml",
            "Cargo.toml",
            "Cargo.lock",
            "crates/unica-coder/**",
            "plugins/unica/**",
            "scripts/ci/**",
            "scripts/install-unica.sh",
            "tests/ci/**",
            "tests/fixtures/**",
            "spec/**",
        ]

        for path in required_paths:
            with self.subTest(path=path):
                self.assertIn(f'- "{path}"', text)

    def test_verify_source_job_runs_full_guardrail_suite(self) -> None:
        text = self.workflow_text()
        required_tokens = [
            "verify-source:",
            "uses: actions/checkout@v4",
            "uses: actions/setup-python@v5",
            'python-version: "3.12"',
            "python -m pip install -r tests/ci/requirements.txt",
            "uses: dtolnay/rust-toolchain@stable",
            "python -m unittest discover -s tests/ci",
            "python -m py_compile scripts/ci/*.py tests/ci/*.py",
            "python -m json.tool plugins/unica/.codex-plugin/plugin.json >/dev/null",
            "python -m json.tool plugins/unica/.mcp.json >/dev/null",
            "python -m json.tool plugins/unica/third-party/tools.lock.json >/dev/null",
            "python -m json.tool plugins/unica/third-party/manifest.json >/dev/null",
            "bash -n plugins/unica/scripts/*.sh",
            "cargo fmt --all -- --check",
            "cargo clippy --package unica-coder --all-targets --all-features -- -D warnings",
            "cargo test --package unica-coder",
        ]

        for token in required_tokens:
            with self.subTest(token=token):
                self.assertIn(token, text)

    def test_pull_request_runs_cancel_stale_commits(self) -> None:
        text = self.workflow_text()

        self.assertIn("concurrency:", text)
        self.assertIn("group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}", text)
        self.assertIn("cancel-in-progress: ${{ github.event_name == 'pull_request' }}", text)

    def test_workflow_is_read_only_by_default(self) -> None:
        text = self.workflow_text()

        self.assertIn("permissions:\n  contents: read", text)
        self.assertNotIn("permissions:\n  contents: write\n\njobs:", text)

    def test_release_artifact_builds_are_gated_on_prs(self) -> None:
        text = self.workflow_text()

        self.assertIn("classify-changes:", text)
        self.assertIn("release_artifacts: ${{ steps.scope.outputs.release_artifacts }}", text)
        self.assertIn("if: ${{ github.event_name != 'pull_request' || needs.classify-changes.outputs.release_artifacts == 'true' }}", text)
        self.assertIn("python scripts/ci/classify-workflow-changes.py", text)

    def test_build_tools_waits_for_source_verification_and_sets_up_rust(self) -> None:
        text = self.workflow_text()
        self.assertIn("build-tools:", text)
        self.assertIn("needs:\n      - verify-source\n      - classify-changes", text)
        self.assertIn("uses: dtolnay/rust-toolchain@stable", text)
        self.assertIn("python scripts/ci/build-unica-tools.py", text)

    def test_release_publishing_is_separate_from_packaging(self) -> None:
        text = self.workflow_text()

        package_job = text[text.index("  package:") : text.index("  installer:")]
        self.assertNotIn("softprops/action-gh-release", package_job)
        self.assertNotIn("contents: write", package_job)

        self.assertIn("publish-release-assets:", text)
        self.assertIn("publish-installer-asset:", text)
        self.assertIn("if: startsWith(github.ref, 'refs/tags/')", text)
        self.assertIn("permissions:\n      contents: write", text)

    def test_release_assessment_uses_linux_marketplace_package(self) -> None:
        text = self.workflow_text()

        self.assertIn("release-assessment:", text)
        self.assertIn("needs: package", text)
        self.assertIn("name: unica-codex-marketplace-linux-x64", text)
        self.assertIn("python scripts/ci/release-assessment.py", text)
        self.assertIn("--package-archive dist/linux-x64/unica-codex-marketplace-linux-x64.tar.gz", text)
        self.assertIn("--bsp-ref 3.2.1.446", text)
        self.assertIn("name: unica-release-assessment", text)

    def test_pages_publish_waits_for_release_assessment(self) -> None:
        text = self.workflow_text()

        self.assertIn("publish-assessment-pages:", text)
        self.assertIn("needs: release-assessment", text)
        self.assertIn("uses: actions/deploy-pages@v4", text)
        self.assertIn("pages: write", text)
        self.assertIn("id-token: write", text)

    def test_release_assets_wait_for_published_assessment_pages(self) -> None:
        text = self.workflow_text()

        publish_assets = text[text.index("  publish-release-assets:") : text.index("  publish-installer-asset:")]
        self.assertIn("needs:\n      - package\n      - publish-assessment-pages", publish_assets)

        publish_installer = text[text.index("  publish-installer-asset:") :]
        self.assertIn("needs:\n      - installer\n      - publish-assessment-pages", publish_installer)


if __name__ == "__main__":
    unittest.main()
