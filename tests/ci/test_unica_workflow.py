from __future__ import annotations

import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
RELEASE_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-plugin-release.yml"
PUBLISH_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "publish-unica-marketplace.yml"
LEGACY_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-legacy-migration.yml"


def job_block(workflow: str, job_id: str) -> str:
    marker = f"  {job_id}:\n"
    start = workflow.find(marker)
    if start == -1:
        return ""
    next_job = re.search(r"(?m)^  [a-zA-Z0-9_-]+:\n", workflow[start + len(marker) :])
    if next_job is None:
        return workflow[start:]
    end = start + len(marker) + next_job.start()
    return workflow[start:end]


class UnicaWorkflowGuardrailTests(unittest.TestCase):
    def release_text(self) -> str:
        return RELEASE_WORKFLOW.read_text(encoding="utf-8")

    def publish_text(self) -> str:
        return PUBLISH_WORKFLOW.read_text(encoding="utf-8")

    def test_source_gate_checks_the_full_rust_and_python_workspace(self) -> None:
        text = self.release_text()

        self.assertIn("cargo clippy --workspace --all-targets --all-features -- -D warnings", text)
        self.assertIn("cargo test --workspace -- --test-threads=1", text)
        self.assertIn("python -m unittest discover -s tests/ci --durations 20", text)
        self.assertIn("python scripts/ci/check-version-contract.py", text)

    def test_every_pull_request_gets_a_stable_aggregate_gate(self) -> None:
        text = self.release_text()
        trigger = text[text.index("on:\n") : text.index("\npermissions:")]
        gate = job_block(text, "unica-ci")

        self.assertIn("  pull_request:\n", trigger)
        self.assertIn("labeled", trigger)
        self.assertIn("unlabeled", trigger)
        self.assertNotIn("paths:", trigger)
        self.assertIn("name: Unica CI", gate)
        self.assertIn("if: always()", gate)
        self.assertIn("python scripts/ci/evaluate-ci-gate.py", gate)
        for upstream in (
            "classify-changes",
            "verify-source",
            "test-rust-primary",
            "test-rust-platforms",
            "build-tools",
            "package-runtime",
            "package-thin",
            "probe-thin-bootstrap",
            "release-assessment",
            "publish-release-assets",
            "smoke-thin-plugin",
            "verify-published-assets",
        ):
            with self.subTest(upstream=upstream):
                self.assertIn(f"      - {upstream}", gate)

    def test_classifier_exposes_typed_contours_and_ci_full_override(self) -> None:
        text = self.release_text()
        classifier = job_block(text, "classify-changes")

        for output in (
            "rust_changed",
            "platform_changed",
            "toolchain_changed",
            "package_changed",
            "plugin_content_changed",
            "ci_changed",
            "release_required",
        ):
            with self.subTest(output=output):
                self.assertIn(f"      {output}:", classifier)
        self.assertIn("contains(github.event.pull_request.labels.*.name, 'ci:full')", classifier)
        self.assertIn("--force-full", classifier)

    def test_rust_jobs_route_primary_and_platform_contours(self) -> None:
        text = self.release_text()
        source = job_block(text, "verify-source")
        primary = job_block(text, "test-rust-primary")
        platforms = job_block(text, "test-rust-platforms")

        self.assertNotIn("cargo test", source)
        self.assertNotIn("dtolnay/rust-toolchain", source)
        self.assertIn("runs-on: macos-14", primary)
        self.assertIn("rust_changed == 'true'", primary)
        self.assertIn("platform_changed == 'false'", primary)
        self.assertIn("runner: [ubuntu-latest, windows-latest, macos-14]", platforms)
        self.assertIn("platform_changed == 'true'", platforms)
        self.assertIn("toolchain_changed == 'true'", platforms)
        self.assertIn("ci_changed == 'true'", platforms)

    def test_package_contour_and_pr_smoke_do_not_publish_release_assets(self) -> None:
        text = self.release_text()
        build = job_block(text, "build-tools")
        probe = job_block(text, "probe-thin-bootstrap")
        publish = job_block(text, "publish-release-assets")

        self.assertIn("release_required == 'true'", build)
        self.assertIn("ci_changed == 'true'", build)
        self.assertIn("github.event_name == 'pull_request'", probe)
        self.assertIn("github.event_name == 'workflow_dispatch'", probe)
        self.assertIn("if: startsWith(github.ref, 'refs/tags/')", publish)

    def test_javascript_actions_use_node24_compatible_majors(self) -> None:
        release = self.release_text()
        publish = self.publish_text()
        combined = release + publish

        self.assertIn("actions/checkout@v7", combined)
        self.assertIn("actions/setup-python@v7", release)
        self.assertIn("actions/upload-artifact@v7", release)
        self.assertIn("actions/download-artifact@v8", release)
        self.assertIn("softprops/action-gh-release@v3", release)
        for stale in (
            "actions/checkout@v4",
            "actions/setup-python@v5",
            "actions/upload-artifact@v4",
            "actions/download-artifact@v4",
            "softprops/action-gh-release@v2",
        ):
            with self.subTest(stale=stale):
                self.assertNotIn(stale, combined)

    def test_heavy_and_external_jobs_have_timeouts(self) -> None:
        release = self.release_text()
        publish = self.publish_text()

        expected_release_timeouts = {
            "classify-changes": 10,
            "verify-source": 90,
            "test-rust-primary": 60,
            "test-rust-platforms": 60,
            "build-tools": 90,
            "package-runtime": 30,
            "package-thin": 30,
            "probe-thin-bootstrap": 30,
            "release-assessment": 60,
            "publish-release-assets": 15,
            "smoke-thin-plugin": 30,
            "verify-published-assets": 15,
            "unica-ci": 5,
        }
        for job_id, minutes in expected_release_timeouts.items():
            with self.subTest(job_id=job_id):
                self.assertIn(f"timeout-minutes: {minutes}", job_block(release, job_id))

        for job_id in ("stage", "promote"):
            with self.subTest(job_id=job_id):
                self.assertIn("timeout-minutes: 20", job_block(publish, job_id))

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

    def test_release_assets_are_published_without_pages_dependency_and_redownloaded(self) -> None:
        text = self.release_text()
        publish = text[text.index("  publish-release-assets:") : text.index("  verify-published-assets:")]
        verify = text[text.index("  verify-published-assets:") :]

        self.assertNotIn("publish-assessment-pages", publish)
        self.assertIn("needs: package-runtime", publish)
        self.assertIn("softprops/action-gh-release@v3", publish)
        self.assertIn("unica-runtime-*.tar.gz", publish)
        self.assertIn("unica-runtime-*.json", publish)
        self.assertNotIn("install-unica", publish)
        self.assertIn("gh release download", verify)
        self.assertIn("verify-release-assets.py", verify)

    def test_release_notes_are_generated_without_repository_docs(self) -> None:
        text = self.release_text()
        publish = text[text.index("  publish-release-assets:") : text.index("  smoke-thin-plugin:")]

        self.assertIn("generate_release_notes: true", publish)
        self.assertNotIn("body_path:", publish)
        self.assertNotIn("docs/releases", text)

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
