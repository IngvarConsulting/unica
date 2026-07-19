from __future__ import annotations

import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
RELEASE_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-plugin-release.yml"
PUBLISH_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "publish-unica-marketplace.yml"
LEGACY_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-legacy-migration.yml"
HISTORICAL_PLAN = (
    REPO_ROOT / "docs" / "superpowers" / "plans" / "2026-07-19-cross-repo-upgrade-regression.md"
)


class UnicaWorkflowGuardrailTests(unittest.TestCase):
    def release_text(self) -> str:
        return RELEASE_WORKFLOW.read_text(encoding="utf-8")

    def publish_text(self) -> str:
        return PUBLISH_WORKFLOW.read_text(encoding="utf-8")

    def test_source_gate_covers_both_rust_packages_and_full_workspace(self) -> None:
        text = self.release_text()

        self.assertIn('"crates/unica-bootstrap/**"', text)
        self.assertIn('"crates/unica-coder/**"', text)
        self.assertIn("cargo clippy --workspace --all-targets --all-features -- -D warnings", text)
        self.assertIn("cargo test --workspace -- --test-threads=1", text)
        self.assertIn("python -m unittest discover -s tests/ci", text)
        self.assertIn("python scripts/ci/check-version-contract.py", text)

    def test_runtime_matrix_builds_deterministic_assets_and_thin_payload(self) -> None:
        text = self.release_text()

        for target in ("darwin-arm64", "linux-x64", "win-x64"):
            self.assertIn(f"target: {target}", text)
        self.assertIn("name: unica-runtime-${{ matrix.target }}", text)
        self.assertIn("dist/unica-runtime-${{ matrix.target }}.tar.gz", text)
        self.assertIn("scripts/ci/build-unica-tools.py", text)
        self.assertIn("scripts/ci/package-unica-runtime.py", text)
        self.assertIn("scripts/ci/package-unica-plugin.py", text)
        self.assertIn("scripts/ci/smoke-unica-mcp.py", text)
        self.assertIn("--runtime-metadata-root", text)
        self.assertIn("--bootstrap-root", text)
        self.assertIn("name: unica-thin-marketplace", text)
        thin_upload = text[text.index("name: unica-thin-marketplace") :]
        self.assertIn("include-hidden-files: true", thin_upload)
        self.assertNotIn("unica-codex-marketplace-${{ matrix.target }}", text)

    def test_packaged_bootstrap_is_smoked_on_every_supported_host(self) -> None:
        text = self.release_text()

        self.assertIn("probe-thin-bootstrap:", text)
        self.assertIn("smoke-thin-plugin:", text)
        self.assertIn("Probe packaged bootstrap through the downloader", text)
        self.assertIn("Smoke packaged bootstrap against published runtime", text)
        self.assertIn("scripts/ci/smoke-unica-bootstrap.py", text)
        self.assertIn("needs: package-thin", text)
        self.assertIn("needs: [package-thin, publish-release-assets]", text)
        self.assertIn("--expect-download-failure", text)

    def test_v080_source_release_has_no_executable_legacy_migration_jobs(self) -> None:
        release = self.release_text()

        for marker in (
            "legacy-migration-preflight:",
            "test-unica-upgrade.ps1",
            "verify-installers:",
            "  installer:",
            "unica-installer",
            "install-unica.sh",
            "install-unica.ps1",
        ):
            with self.subTest(marker=marker):
                self.assertNotIn(marker, release)

    def test_source_repo_has_no_manual_or_scheduled_full_migration_workflow(self) -> None:
        release = self.release_text()
        violations: dict[str, list[str]] = {}
        workflows = sorted(
            (*WORKFLOWS_DIR.glob("*.yml"), *WORKFLOWS_DIR.glob("*.yaml")),
            key=lambda path: path.name,
        )

        for workflow in workflows:
            text = workflow.read_text(encoding="utf-8")
            markers = [
                marker
                for marker in ("-Mode Full", "legacy-migration-full")
                if marker in text
            ]
            if markers:
                violations[workflow.name] = markers

        self.assertFalse(LEGACY_WORKFLOW.exists())
        self.assertNotIn("unica-legacy-migration.yml", release)
        self.assertEqual({}, violations, f"source workflows own full migration policy: {violations}")

    def test_historical_plan_references_the_actual_workflow_test_module(self) -> None:
        text = HISTORICAL_PLAN.read_text(encoding="utf-8")

        self.assertIn("tests/ci/test_unica_workflow.py", text)
        self.assertIn("tests.ci.test_unica_workflow -v", text)
        self.assertNotIn("test_unica_workflows", text)

    def test_release_assets_are_published_without_pages_dependency_and_redownloaded(self) -> None:
        text = self.release_text()
        publish = text[text.index("  publish-release-assets:") : text.index("  verify-published-assets:")]
        verify = text[text.index("  verify-published-assets:") :]

        self.assertNotIn("publish-assessment-pages", publish)
        self.assertIn("needs: package-runtime", publish)
        self.assertIn("softprops/action-gh-release@v2", publish)
        self.assertIn("unica-runtime-*.tar.gz", publish)
        self.assertIn("unica-runtime-*.json", publish)
        self.assertNotIn("install-unica", publish)
        self.assertIn("gh release download", verify)
        self.assertIn("verify-release-assets.py", verify)

    def test_assessment_is_independent_from_runtime_publication(self) -> None:
        text = self.release_text()
        assessment = text[text.index("  release-assessment:") : text.index("  publish-release-assets:")]

        self.assertIn("always()", assessment)
        self.assertIn("unica-runtime-linux-x64.tar.gz", assessment)
        self.assertNotIn("publish-release-assets", assessment)
        self.assertIn("if: always()", text[text.index("name: unica-release-assessment") - 120 :])

    def test_pr_permissions_are_read_only_and_cross_repo_write_uses_secret(self) -> None:
        release = self.release_text()
        publish = self.publish_text()

        self.assertIn("permissions:\n  contents: read", release)
        self.assertIn("permissions:\n  contents: read", publish)
        self.assertIn("UNICA_MARKETPLACE_TOKEN", publish)
        self.assertIn("GH_TOKEN: ${{ secrets.UNICA_MARKETPLACE_TOKEN }}", publish)
        self.assertNotIn("pull-requests: write", publish)

    def test_cross_repository_push_configures_git_credentials(self) -> None:
        publish = self.publish_text()

        self.assertGreaterEqual(publish.count("gh auth setup-git"), 2)

    def test_staging_and_promotion_are_explicit_separate_jobs(self) -> None:
        text = self.publish_text()

        self.assertIn("workflow_run:", text)
        self.assertIn("workflow_dispatch:", text)
        self.assertIn("mode:", text)
        self.assertIn("staging_merge_sha:", text)
        self.assertIn("stage:", text)
        self.assertIn("promote:", text)
        self.assertIn("codex/stage-", text)
        self.assertIn("codex/promote-", text)
        self.assertIn('git -C marketplace merge-base --is-ancestor "$STAGING_MERGE_SHA" "origin/main"', text)
        self.assertIn('promotion_sha="$(git -C marketplace rev-parse HEAD)"', text)
        self.assertIn("Create the signed ${RELEASE_TAG} tag at commit ${promotion_sha}", text)
        self.assertNotIn("git ls-remote", text)
        self.assertNotIn('"refs/tags/${RELEASE_TAG}^{}"', text)
        self.assertIn("payload/plugins/unica/.codex-plugin/plugin.json", text)
        self.assertIn("payload/plugins/unica/.mcp.json", text)
        self.assertIn("payload/.agents/plugins/marketplace.json", text)
        self.assertNotIn("git tag -f", text)
        self.assertNotIn("--force", text)


if __name__ == "__main__":
    unittest.main()
