from __future__ import annotations

import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
RELEASE_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-plugin-release.yml"
PUBLISH_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "publish-unica-marketplace.yml"
LEGACY_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-legacy-migration.yml"


RUN_KEY = re.compile(r"^(?P<lead>\s*(?:- )?)run:(?P<inline>.*)$")


def run_block_lines(workflow: str):
    """Yield `(line number, text)` for every line that becomes shell script.

    A `run:` value is the only place in a workflow where text is handed to a
    shell, so it is the only place where an interpolated `${{ }}` is script
    rather than data. `if:`, `concurrency:` and `env:` values are evaluated by
    GitHub itself and never parsed by bash, which is exactly why binding a ref
    name to `env:` defuses it.

    Blocks are found by indentation: the body of a block scalar is indented
    past the column of its `run:` key. PyYAML is a suite dependency for skill
    frontmatter, but this guard inspects source spelling so YAML normalization
    cannot hide which bytes become shell script.
    """
    lines = workflow.splitlines()
    index = 0
    while index < len(lines):
        match = RUN_KEY.match(lines[index])
        if match is None:
            index += 1
            continue

        key_column = len(match.group("lead"))
        if match.group("inline").strip() not in ("", "|", ">", "|-", ">-"):
            yield index + 1, lines[index]
        index += 1

        while index < len(lines):
            body = lines[index]
            if body.strip() and len(body) - len(body.lstrip()) <= key_column:
                break
            if body.strip():
                yield index + 1, body
            index += 1


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
        self.assertIn("python -m unittest discover -s tests/dev --durations 20", text)
        self.assertIn("python -m py_compile scripts/dev/*.py tests/dev/*.py", text)
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
            "test-search-integration",
            "build-tools",
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
            "search_integration_changed",
            "package_changed",
            "plugin_content_changed",
            "ci_changed",
            "release_required",
        ):
            with self.subTest(output=output):
                self.assertIn(f"      {output}:", classifier)
        self.assertIn("contains(github.event.pull_request.labels.*.name, 'ci:full')", classifier)
        self.assertIn("--force-full", classifier)

    def test_classifier_preserves_merge_base_for_triple_dot_diff(self) -> None:
        text = self.release_text()
        classifier = job_block(text, "classify-changes")

        self.assertIn("fetch-depth: 0", classifier)
        self.assertIn("BASE_REF: ${{ github.base_ref }}", classifier)
        self.assertIn('git fetch --no-tags origin "$BASE_REF"', classifier)
        self.assertNotIn("--depth", classifier)
        self.assertIn("FORCE_FULL", classifier)
        self.assertIn("git diff --name-only FETCH_HEAD...HEAD", classifier)

    def test_a_ref_name_never_reaches_the_shell_as_script_text(self) -> None:
        """A ref name is data, so it crosses into `run:` through `env:`.

        Git ref rules forbid spaces but allow `;`, `$`, backticks and
        parentheses, so a ref name pasted into a `run:` block through `${{ }}`
        is a command-injection sink. Passing it as an environment variable keeps
        the shell from ever parsing it.

        Branch and tag names are the same sink: `github.ref_name` obeys exactly
        the rules `github.base_ref` does, so pinning only the name the review
        happened to cite would leave the defect a second way back in.
        """
        refs = ("github.base_ref", "github.ref_name", "github.head_ref")
        scanned = 0
        # GitHub accepts either extension, so a guard that scans one of them
        # leaves the other as a blind spot.
        for workflow in sorted(
            (*WORKFLOWS_DIR.glob("*.yml"), *WORKFLOWS_DIR.glob("*.yaml")),
            key=lambda path: path.name,
        ):
            for number, line in run_block_lines(workflow.read_text(encoding="utf-8")):
                scanned += 1
                context = next((ref for ref in refs if ref in line), None)
                with self.subTest(workflow=workflow.name, line=number):
                    self.assertIsNone(
                        context,
                        f"{context} is interpolated into a run block; bind it "
                        "to an env variable and read it as \"$NAME\"",
                    )

        # An indentation scanner that silently matched nothing would pass this
        # test forever. The workflows have far more shell than this.
        self.assertGreater(scanned, 50)

    def test_rust_jobs_route_primary_and_platform_contours(self) -> None:
        text = self.release_text()
        source = job_block(text, "verify-source")
        primary = job_block(text, "test-rust-primary")
        platforms = job_block(text, "test-rust-platforms")
        search_integration = job_block(text, "test-search-integration")

        self.assertNotIn("cargo test", source)
        self.assertIn("search_integration_changed == 'true'", search_integration)
        self.assertIn("--test issue_89_workspace_service -- --ignored", search_integration)
        self.assertNotIn("dtolnay/rust-toolchain", source)
        self.assertIn("runs-on: macos-14", primary)
        self.assertIn("rust_changed == 'true'", primary)
        self.assertIn("platform_changed == 'false'", primary)
        self.assertIn("runner: [ubuntu-latest, windows-latest, macos-14]", platforms)
        self.assertIn("platform_changed == 'true'", platforms)
        self.assertIn("toolchain_changed == 'true'", platforms)
        self.assertIn("ci_changed == 'true'", platforms)
        self.assertEqual(2, platforms.count("if: matrix.runner == 'macos-14'"))

    def test_package_contour_and_pr_smoke_do_not_publish_release_assets(self) -> None:
        text = self.release_text()
        build = job_block(text, "build-tools")
        probe = job_block(text, "probe-thin-bootstrap")
        publish = job_block(text, "publish-release-assets")

        self.assertIn("release_required == 'true'", build)
        self.assertIn("ci_changed == 'true'", build)
        self.assertIn("github.event_name == 'pull_request'", probe)
        self.assertIn("github.event_name == 'workflow_dispatch'", probe)
        self.assertIn("startsWith(github.ref, 'refs/tags/')", publish)

    def test_only_tag_pushes_enable_release_behavior(self) -> None:
        text = self.release_text()
        build = job_block(text, "build-tools")
        thin = job_block(text, "package-thin")

        self.assertIn(
            "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/')",
            build,
        )
        self.assertIn(
            "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/')",
            thin,
        )
        for job_id in ("publish-release-assets", "smoke-thin-plugin", "verify-published-assets"):
            with self.subTest(job_id=job_id):
                job = job_block(text, job_id)
                self.assertIn("github.event_name == 'push'", job)
                self.assertIn("startsWith(github.ref, 'refs/tags/')", job)

    def test_conditional_pipeline_breaks_transitive_skip_propagation(self) -> None:
        text = self.release_text()
        dependencies = {
            "package-thin": ("needs.build-tools.result == 'success'",),
            "probe-thin-bootstrap": ("needs.package-thin.result == 'success'",),
            "release-assessment": ("needs.build-tools.result == 'success'",),
            "publish-release-assets": ("needs.build-tools.result == 'success'",),
            "smoke-thin-plugin": (
                "needs.package-thin.result == 'success'",
                "needs.publish-release-assets.result == 'success'",
            ),
            "verify-published-assets": (
                "needs.package-thin.result == 'success'",
                "needs.publish-release-assets.result == 'success'",
            ),
        }

        for job_id, dependency_results in dependencies.items():
            with self.subTest(job_id=job_id):
                job = job_block(text, job_id)
                self.assertIn("always()", job)
                for dependency_result in dependency_results:
                    self.assertIn(dependency_result, job)

    def test_javascript_actions_use_node24_compatible_majors(self) -> None:
        release = self.release_text()
        publish = self.publish_text()
        combined = release + publish

        self.assertIn("actions/checkout@v7", combined)
        self.assertIn("actions/setup-python@v7", release)
        self.assertIn("actions/cache@v5", release)
        self.assertIn("actions/upload-artifact@v7", release)
        self.assertIn("actions/download-artifact@v8", release)
        self.assertIn("softprops/action-gh-release@v3", release)
        for stale in (
            "actions/checkout@v4",
            "actions/setup-python@v5",
            "actions/cache@v4",
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

    def test_platform_build_uses_exact_cargo_cache_and_reports_outcome(self) -> None:
        text = self.release_text()
        build = job_block(text, "build-tools")

        self.assertIn("id: rust-toolchain", build)
        self.assertIn("id: cargo-cache", build)
        self.assertIn("continue-on-error: true", build)
        self.assertIn("uses: actions/cache@v5", build)
        self.assertIn("path: .build/tool-work/${{ matrix.target }}/cargo-target", build)
        self.assertIn(
            "key: cargo-${{ runner.os }}-${{ matrix.target }}-${{ "
            "steps.rust-toolchain.outputs.cachekey }}-${{ hashFiles('Cargo.lock') }}",
            build,
        )
        self.assertNotIn("restore-keys:", build)
        self.assertLess(build.index("id: cargo-cache"), build.index("scripts/ci/build-unica-tools.py"))
        self.assertIn("--metrics-file", build)
        self.assertIn("if: always()", build)
        self.assertIn("steps.cargo-cache.outcome", build)
        self.assertIn("steps.cargo-cache.outputs.cache-hit", build)
        for outcome in ("exact-hit", "miss", "error"):
            with self.subTest(outcome=outcome):
                self.assertIn(outcome, build)
        self.assertIn("cargoBuildSeconds", build)
        self.assertIn("GITHUB_STEP_SUMMARY", build)

    def test_runtime_matrix_builds_verifies_and_exports_narrow_artifacts(self) -> None:
        text = self.release_text()
        build = job_block(text, "build-tools")

        for target in ("darwin-arm64", "linux-x64", "win-x64"):
            self.assertIn(f"target: {target}", text)
        self.assertNotIn("  package-runtime:\n", text)
        self.assertNotIn("unica-tools-", text)
        self.assertIn("scripts/ci/build-unica-tools.py", build)
        self.assertIn("scripts/ci/package-unica-runtime.py", build)
        self.assertIn("scripts/ci/verify-release-assets.py", build)
        self.assertIn('--target "${{ matrix.target }}"', build)
        self.assertIn("name: unica-runtime-metadata-${{ matrix.target }}", build)
        self.assertIn("name: unica-bootstrap-${{ matrix.target }}", build)
        self.assertIn("name: unica-runtime-${{ matrix.target }}", text)
        self.assertIn(
            ".build/runtime-assets/${{ matrix.target }}/unica-runtime-${{ matrix.target }}.json",
            build,
        )
        self.assertIn(
            ".build/runtime-assets/${{ matrix.target }}/unica-runtime-${{ matrix.target }}.tar.gz",
            build,
        )
        self.assertIn(
            ".build/bootstrap-artifacts/${{ matrix.target }}/bootstrap/bin/${{ matrix.target }}",
            build,
        )
        self.assertIn("matrix.target == 'linux-x64'", build)
        self.assertIn("startsWith(github.ref, 'refs/tags/')", build)
        self.assertGreaterEqual(build.count("retention-days: 1"), 3)

    def test_thin_payload_downloads_only_metadata_and_bootstrap(self) -> None:
        text = self.release_text()
        thin = job_block(text, "package-thin")

        self.assertIn("needs: build-tools", thin)
        self.assertIn("pattern: unica-runtime-metadata-*", thin)
        self.assertIn("pattern: unica-bootstrap-*", thin)
        self.assertNotIn("pattern: unica-tools-*", thin)
        self.assertNotIn("pattern: unica-runtime-*\n", thin)
        self.assertIn("scripts/ci/package-unica-plugin.py", text)
        self.assertIn("--runtime-metadata-root", thin)
        self.assertIn("--bootstrap-root", thin)
        self.assertIn("name: unica-thin-marketplace", thin)
        self.assertIn("include-hidden-files: true", thin)
        self.assertIn("retention-days: 90", thin)
        self.assertNotIn("unica-codex-marketplace-${{ matrix.target }}", text)

    def test_intermediate_non_marketplace_artifacts_expire_after_one_day(self) -> None:
        text = self.release_text()
        assessment = job_block(text, "release-assessment")

        self.assertIn("name: unica-release-assessment", assessment)
        self.assertIn("retention-days: 1", assessment)

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
        self.assertIn("needs: build-tools", publish)
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
        # The tag names the staging merge, which already carries the plugin bytes
        # and exists before this job runs. The promotion commit does not, so
        # naming it kept the consumer install checks red on their first run.
        self.assertIn(
            "tag at the staging merge commit ${STAGING_MERGE_SHA} before merging this PR",
            text,
        )
        self.assertNotIn("${promotion_sha}", text)
        self.assertNotIn("git ls-remote", text)
        self.assertNotIn('"refs/tags/${RELEASE_TAG}^{}"', text)
        self.assertIn("payload/plugins/unica/.codex-plugin/plugin.json", text)
        self.assertIn("payload/plugins/unica/.mcp.json", text)
        self.assertIn("payload/.agents/plugins/marketplace.json", text)
        self.assertNotIn("git tag -f", text)
        self.assertNotIn("--force", text)


if __name__ == "__main__":
    unittest.main()
