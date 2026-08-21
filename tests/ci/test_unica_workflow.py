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
            "assessment_required",
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
        self.assertIn("ci_changed == 'true'", search_integration)
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

    def test_search_integration_checkout_does_not_persist_credentials(self) -> None:
        integration = job_block(self.release_text(), "test-search-integration")

        self.assertIn(
            "      - uses: actions/checkout@v7\n"
            "        with:\n"
            "          persist-credentials: false",
            integration,
        )

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

    def test_release_assessment_uses_affected_mechanism_contour(self) -> None:
        text = self.release_text()
        assessment = job_block(text, "release-assessment")

        self.assertIn("needs: [classify-changes, build-tools]", assessment)
        self.assertIn("assessment_required == 'true'", assessment)
        self.assertIn("needs.build-tools.result == 'success'", assessment)

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

        expected_publish_timeouts = {
            "stage": 20,
            "tag": 10,
            "verify-fresh-install": 30,
            "verify-upgrade": 30,
            "promote": 10,
        }
        for job_id, minutes in expected_publish_timeouts.items():
            with self.subTest(job_id=job_id):
                self.assertIn(f"timeout-minutes: {minutes}", job_block(publish, job_id))

    def test_registry_guards_run_in_the_source_contour(self) -> None:
        verify = job_block(self.release_text(), "verify-source")

        self.assertIn("python -m unittest discover -s tests/arch", verify)
        self.assertNotIn("python -m unittest discover -s tests/arch -t .", verify)
        self.assertIn("python -m py_compile scripts/arch/*.py tests/arch/*.py", verify)

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
        self.assertIn("archiveDownloadSeconds", build)
        self.assertIn("RLM archive download duration", build)
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
        # Узость здесь — про цель, а не про артефакт: разрез поставки дал по
        # архиву на артефакт, и выгрузка обязана нести их все.
        self.assertIn(
            ".build/runtime-assets/${{ matrix.target }}/*-runtime-${{ matrix.target }}.json",
            build,
        )
        self.assertIn(
            ".build/runtime-assets/${{ matrix.target }}/*-runtime-${{ matrix.target }}.tar.gz",
            build,
        )
        self.assertIn(
            ".build/bootstrap-artifacts/${{ matrix.target }}/bootstrap/bin/${{ matrix.target }}",
            build,
        )
        self.assertIn("matrix.target == 'linux-x64'", build)
        self.assertIn("startsWith(github.ref, 'refs/tags/')", build)
        self.assertGreaterEqual(build.count("retention-days: 1"), 3)

    def test_mcp_smoke_runs_against_extracted_deterministic_runtime(self) -> None:
        build = job_block(self.release_text(), "build-tools")

        package = build.index("name: Package deterministic runtime")
        extract = build.index("name: Extract deterministic runtime for MCP smoke")
        smoke = build.index("name: Smoke packaged Unica MCP")
        stage = build.index("name: Stage exact bootstrap payload", smoke)
        smoke_step = build[smoke:stage]
        self.assertLess(package, extract)
        self.assertLess(extract, smoke)
        self.assertIn('runtime_root=".build/runtime-smoke/${{ matrix.target }}"', build)
        self.assertIn(
            'tar -xzf ".build/runtime-assets/${{ matrix.target }}/unica-runtime-${{ matrix.target }}.tar.gz"',
            build,
        )
        self.assertIn('--plugin-root "$runtime_root"', build)
        self.assertIn('executable="$runtime_root/bin/${{ matrix.target }}/unica"', build)
        self.assertIn("timeout-minutes: 3", smoke_step)
        self.assertIn("--total-timeout-seconds 120", smoke_step)

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

    def test_publication_is_one_linear_pass_ordered_by_needs(self) -> None:
        """ADR-0068: stage → tag → verify → promote, no pull requests, no warden.

        The order is the contract: the anchor tag exists before the install
        checks run, and the catalog moves only behind their green result. A
        rerun of the whole workflow resumes a partial publication, so every
        stage states its idempotent escape.
        """
        text = self.publish_text()

        self.assertIn("workflow_run:", text)
        self.assertIn("workflow_dispatch:", text)
        self.assertIn("source_run_id:", text)
        for job in ("stage:", "tag:", "verify-fresh-install:", "verify-upgrade:", "promote:"):
            self.assertIn(f"\n  {job}", text)
        self.assertIn("needs: stage", text)
        self.assertIn("needs: [stage, tag]", text)
        self.assertIn("needs: [stage, tag, verify-fresh-install, verify-upgrade]", text)
        # The PR ceremony is gone with the warden: nothing opens pull requests
        # and no metadata travels in branch names.
        self.assertNotIn("pr create", text)
        self.assertNotIn("codex/stage-", text)
        self.assertNotIn("codex/promote-", text)
        self.assertNotIn("mode:", text)
        # Idempotent escapes: a completed stage and a completed promote are
        # detected, and an existing tag is proven identical, never moved.
        self.assertEqual(text.count("diff --cached --quiet"), 2)
        self.assertIn('rev-parse --verify --quiet "refs/tags/${RELEASE_TAG}"', text)
        self.assertNotIn("git tag -f", text)
        self.assertNotIn("--force", text)
        # Two releases must not interleave, and a stale straggler must fail
        # forward-only instead of rolling the catalog back — in both writers,
        # over both host catalogs, and again after a rebase retry in promote.
        self.assertIn("group: publish-unica-marketplace", text)
        self.assertIn("cancel-in-progress: false", text)
        self.assertEqual(text.count("require_forward()"), 2)
        self.assertEqual(text.count('test "$newest" = "$RELEASE_TAG"'), 2)
        self.assertEqual(
            text.count(".agents/plugins/marketplace.json .claude-plugin/marketplace.json"),
            3,  # both guard loops and the promote `git add`
        )
        self.assertIn('require_forward "HEAD~1"', text)
        # The payload is trusted only from the successful push build of the
        # very tag its manifest declares — dispatch cannot smuggle another one.
        self.assertIn('test "$run_event" = "push"', text)
        self.assertIn('test "$run_branch" = "$RELEASE_TAG"', text)
        self.assertIn('gh api "repos/IngvarConsulting/unica/git/ref/tags/${RELEASE_TAG}" --silent', text)
        self.assertIn("payload/plugins/unica/.codex-plugin/plugin.json", text)
        self.assertIn("payload/plugins/unica/.mcp.json", text)
        self.assertIn("payload/.agents/plugins/marketplace.json", text)
        # Consumer verification installs the candidate the way a consumer does,
        # on every supported host, before the catalog moves.
        self.assertEqual(text.count("plugin marketplace add $candidate --json"), 2)
        # The upgrade gate seeds the previous stable and then moves that same
        # install to the candidate. The candidate is a directory marketplace, so
        # the move is a reinstall against the rewritten catalog: `plugin
        # marketplace upgrade` fetches a Git remote and refuses one.
        self.assertNotIn("plugin marketplace upgrade unica", text)
        self.assertIn("plugin remove unica@unica --json", text)
        self.assertEqual(text.count("plugin add unica@unica --json"), 3)
        self.assertIn("verify --plugin-root $pluginRoot", text)


class ArtifactSplitPublicationTests(unittest.TestCase):
    """Разрез поставки делит сборку и выкладку по-разному.

    Сборка несёт все артефакты: их метаданные нужны упаковщику, чтобы манифест
    объявил каждый. Выкладка несёт одно ядро: движки издал тулчейн, и вторая
    публикация тех же байтов стоила 439 МБ на выпуск.
    """

    def setUp(self) -> None:
        self.release = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        self.publish = PUBLISH_WORKFLOW.read_text(encoding="utf-8")

    def test_the_release_publishes_the_core_and_only_it(self) -> None:
        # Выкладывается то, у чего есть читатель: пару ядра перекачивает и
        # перехеширует `verify-release-assets.py`. Описания поставок читает
        # только упаковщик, и берёт он их из артефакта сборки.
        self.assertIn("dist/runtime/unica-runtime-*.tar.gz", self.release)
        self.assertIn("dist/runtime/unica-runtime-*.json", self.release)
        self.assertNotIn("dist/runtime/*-runtime-*", self.release)

    def test_the_manifest_still_names_the_artifacts_the_release_does_not_carry(
        self,
    ) -> None:
        # Не выложить и не назвать — разные вещи. Движки объявлены адресом, и
        # каждый адрес выпуск проверяет.
        self.assertIn("verify-delivery-reachable.py", self.release)
        self.assertIn("prefetch --plugin-root", self.release)

    def test_packaging_uploads_every_artifact_of_the_target(self) -> None:
        for glob in (
            "runtime-assets/${{ matrix.target }}/*-runtime-${{ matrix.target }}.tar.gz",
            "runtime-assets/${{ matrix.target }}/*-runtime-${{ matrix.target }}.json",
        ):
            self.assertIn(glob, self.release, glob)

    def test_bsp_runtime_assessment_receives_the_engine_its_search_requires(self) -> None:
        build = job_block(self.release, "build-tools")
        assessment = job_block(self.release, "release-assessment")

        self.assertIn("unica-assessment-engine-linux-x64", build)
        self.assertIn("stage-unica-assessment-engine.py", build)
        self.assertIn("--artifact bsl-analyzer", build)
        self.assertIn("--artifact rlm-tools-bsl", build)
        self.assertIn("--out-archive .build/unica-assessment-engine-linux-x64.tar.gz", build)
        self.assertIn("name: unica-assessment-engine-linux-x64", assessment)
        self.assertIn(
            "--engine-overlay .build/assessment-engine/unica-assessment-engine-linux-x64.tar.gz",
            assessment,
        )

    def test_the_direct_mcp_smoke_is_given_the_engines_it_asserts_on(self) -> None:
        build = job_block(self.release, "build-tools")
        extract = build[
            build.index("name: Extract deterministic runtime for MCP smoke") :
        ].split("- name: Smoke packaged Unica MCP")[0]

        self.assertIn("unica-runtime-${{ matrix.target }}.tar.gz", extract)
        self.assertIn(".build/tool-bundles/${{ matrix.target }}/bin/", extract)

    def test_every_supported_target_must_pass_before_publication(self) -> None:
        expected_targets = (
            "          - target: linux-x64\n            runner: ubuntu-latest",
            "          - target: win-x64\n            runner: windows-latest",
            "          - target: darwin-arm64\n            runner: macos-14",
        )
        build = job_block(self.release, "build-tools")
        probe = job_block(self.release, "probe-thin-bootstrap")
        smoke = job_block(self.release, "smoke-thin-plugin")
        package = job_block(self.release, "package-thin")
        publish = job_block(self.release, "publish-release-assets")
        verification = job_block(self.release, "verify-published-assets")

        for target in expected_targets:
            with self.subTest(contour="build", target=target):
                self.assertIn(target, build)
        for target in (
            expected_targets[0],
            expected_targets[1].replace("windows-latest", "windows-2022"),
            expected_targets[2],
        ):
            with self.subTest(contour="probe", target=target):
                self.assertIn(target, probe)
            with self.subTest(contour="published-smoke", target=target):
                self.assertIn(target, smoke)

        self.assertIn("needs: build-tools", package)
        self.assertIn("needs.build-tools.result == 'success'", package)
        self.assertIn("needs: build-tools", publish)
        self.assertIn("needs.build-tools.result == 'success'", publish)
        self.assertIn("needs: package-thin", probe)
        self.assertIn("needs.package-thin.result == 'success'", probe)
        self.assertIn("needs: [package-thin, publish-release-assets]", smoke)
        self.assertIn("needs.package-thin.result == 'success'", smoke)
        self.assertIn("needs.publish-release-assets.result == 'success'", smoke)
        self.assertIn("      - publish-release-assets", verification)
        self.assertIn("      - package-thin", verification)
        self.assertIn("needs.package-thin.result == 'success'", verification)
        self.assertIn("needs.publish-release-assets.result == 'success'", verification)


class PrereleaseNeverReachesConsumersTests(unittest.TestCase):
    """Предвыпуск собирается и публикует ассеты, но каталога не касается.

    Замерить доставку можно только на настоящем релизе: адрес архива прибит к
    релизам репозитория. Значит нужен выпуск, который существует для нас и не
    существует для пользователей, — и решать это должен конвейер, а не память
    того, кто его запускал.
    """

    def setUp(self) -> None:
        self.release = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        self.publish = PUBLISH_WORKFLOW.read_text(encoding="utf-8")

    def test_a_prerelease_tag_marks_the_github_release_as_such(self) -> None:
        # Иначе предвыпуск станет «последним релизом» и его начнут находить
        # те, кто ищет свежее.
        self.assertIn("prerelease: ${{ contains(github.ref_name, '-') }}", self.release)

    def test_publication_asks_first_whether_this_release_is_for_consumers(self) -> None:
        self.assertIn("\n  gate:\n", self.publish)
        self.assertIn("promote:", self.publish)

    def test_every_publishing_stage_waits_for_that_answer(self) -> None:
        # Достаточно загейтить первую стадию: остальные ждут её через `needs`.
        self.assertIn("needs: gate", self.publish)
        self.assertIn("if: needs.gate.outputs.promote == 'true'", self.publish)


if __name__ == "__main__":
    unittest.main()
