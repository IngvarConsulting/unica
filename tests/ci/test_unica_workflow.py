from __future__ import annotations

import re
import unittest
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
RELEASE_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-plugin-release.yml"
NIGHTLY_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-nightly.yml"
PAGES_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-pages.yml"
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


@dataclass(frozen=True)
class ParsedJob:
    body: str
    needs: tuple[str, ...]
    targets: tuple[tuple[str, str], ...]
    steps: tuple[str, ...]


def parse_workflow_jobs(workflow: str) -> dict[str, ParsedJob]:
    """Parse the job graph and matrix/step order from the workflow subset we own."""
    lines = workflow.splitlines()
    jobs_start = lines.index("jobs:") + 1
    boundaries = [
        index
        for index in range(jobs_start, len(lines))
        if re.fullmatch(r"  [A-Za-z0-9_-]+:", lines[index])
    ]
    jobs: dict[str, ParsedJob] = {}
    for position, start in enumerate(boundaries):
        end = boundaries[position + 1] if position + 1 < len(boundaries) else len(lines)
        name = lines[start].strip()[:-1]
        block_lines = lines[start:end]
        body = "\n".join(block_lines) + "\n"
        needs: list[str] = []
        for index, line in enumerate(block_lines):
            match = re.fullmatch(r"    needs:\s*(.*)", line)
            if not match:
                continue
            raw = match.group(1).strip()
            if raw.startswith("["):
                needs.extend(item.strip() for item in raw[1:-1].split(",") if item.strip())
            elif raw:
                needs.append(raw)
            else:
                cursor = index + 1
                while cursor < len(block_lines):
                    item = re.fullmatch(r"      - ([A-Za-z0-9_-]+)", block_lines[cursor])
                    if item is None:
                        break
                    needs.append(item.group(1))
                    cursor += 1
            break
        targets = tuple(
            (target, runner)
            for target, runner in re.findall(
                r"(?m)^          - target: ([^\s]+)\n            runner: ([^\s]+)$",
                body,
            )
        )
        steps = tuple(
            match.group(1).strip('"\'')
            for line in block_lines
            if (match := re.fullmatch(r"      - (?:name|uses): (.+)", line))
        )
        jobs[name] = ParsedJob(body=body, needs=tuple(needs), targets=targets, steps=steps)
    return jobs


class UnicaWorkflowGuardrailTests(unittest.TestCase):
    def release_text(self) -> str:
        return RELEASE_WORKFLOW.read_text(encoding="utf-8")

    def nightly_text(self) -> str:
        return NIGHTLY_WORKFLOW.read_text(encoding="utf-8")

    def pages_text(self) -> str:
        return PAGES_WORKFLOW.read_text(encoding="utf-8")

    def publish_text(self) -> str:
        return PUBLISH_WORKFLOW.read_text(encoding="utf-8")

    def test_source_gate_checks_the_full_rust_and_python_workspace(self) -> None:
        text = self.release_text()

        self.assertIn("cargo clippy --workspace --all-targets --all-features --message-format=json -- -D warnings", text)
        # Наборы гоняет шов; сами команды закреплены тестом `test_run_tests`.
        self.assertIn('python3 scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem rust --results', text)
        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem python --results', text)
        self.assertNotIn("cargo test --workspace", text)
        self.assertNotIn("unittest discover", text)
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
            "guards",
            "test-python",
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

    def test_p0_dry_release_proof_is_read_only_and_aggregated(self) -> None:
        text = self.release_text()
        jobs = parse_workflow_jobs(text)
        proof = jobs.get("p0-release-proof")
        self.assertIsNotNone(proof)
        assert proof is not None
        self.assertEqual(
            set(proof.needs),
            {"build-tools", "package-thin", "release-assessment"},
        )
        for argument in (
            "scripts/ci/release-proof.py",
            "--mode dry",
            "--wire-dir",
            "--package-dir",
            "--asset-verification-dir",
            "--source-commit",
            "--baseline",
            "--out-dir dist/p0-proof",
        ):
            self.assertIn(argument, proof.body)
        self.assertIn("permissions:\n      contents: read", proof.body)
        self.assertNotIn("softprops/action-gh-release", proof.body)
        self.assertNotIn("git tag", proof.body)
        self.assertIn("      - p0-release-proof", job_block(text, "unica-ci"))

    def test_wire_probes_embed_the_matrix_target_in_their_evidence(self) -> None:
        build = job_block(self.release_text(), "build-tools")

        self.assertEqual(2, build.count('--target "$TARGET"'))

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
        # Имя ветки прогона-источника приходит из того же класса значений:
        # его выбирает тот, кто может завести ветку или тег.
        refs = (
            "github.base_ref",
            "github.ref_name",
            "github.head_ref",
            "github.event.workflow_run.head_branch",
        )
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
        source = job_block(text, "test-python")
        primary = job_block(text, "test-rust-primary")
        platforms = job_block(text, "test-rust-platforms")
        search_integration = job_block(text, "test-search-integration")

        self.assertNotIn("cargo test", source)
        self.assertIn("search_integration_changed == 'true'", search_integration)
        self.assertIn("ci_changed == 'true'", search_integration)
        self.assertIn("--test v13_search_integration -- --ignored", search_integration)
        self.assertNotIn("dtolnay/rust-toolchain", source)
        self.assertIn("runs-on: macos-14", primary)
        self.assertIn("rust_changed == 'true'", primary)
        self.assertIn("platform_changed == 'false'", primary)
        # windows-latest снят с матрицы до разбора нестабильности раннера;
        # список закреплён целиком, поэтому вернуть его молча не получится.
        self.assertIn("runner: [ubuntu-latest, macos-14]", platforms)
        self.assertIn("platform_changed == 'true'", platforms)
        self.assertIn("toolchain_changed == 'true'", platforms)
        self.assertIn("ci_changed == 'true'", platforms)
        # Форматирование от цели не зависит — оно в `guards`. Линт зависит:
        # `#[cfg]` решает, какие элементы существуют, поэтому clippy идёт на
        # каждом раннере матрицы, а его находки — в Code Scanning.
        self.assertNotIn("cargo fmt", platforms)
        self.assertNotIn("cargo fmt", primary)
        self.assertIn("cargo fmt --all -- --check", job_block(text, "guards"))
        for block, category in ((platforms, "clippy-${{ matrix.runner }}"), (primary, "clippy-macos-14-primary")):
            self.assertIn("| clippy-sarif | tee clippy.sarif | sarif-fmt", block)
            self.assertIn(f"category: {category}", block)
            self.assertIn("needs: [classify-changes, guards]", block)
        # Команда линта закреплена дословно: `-D warnings` красит джобу, JSON
        # идёт в SARIF, и убрать одно из двух молча не выйдет.
        self.assertIn(
            "      - run: |\n"
            "          set -o pipefail\n"
            "          cargo clippy --workspace --all-targets --all-features"
            " --message-format=json -- -D warnings \\\n"
            "            | clippy-sarif | tee clippy.sarif | sarif-fmt\n",
            platforms,
        )

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
        # Сборка и холодный старт сняты с pull request и с push в ветку: там они
        # не давали прослеживаемости, а гейт красили. Тег и ручной запуск их
        # сохраняют.
        self.assertIn("(github.event_name == 'workflow_dispatch' || startsWith(github.ref, 'refs/tags/'))", build)
        self.assertNotIn("github.event_name != 'pull_request'", build)
        self.assertIn("github.event_name == 'workflow_dispatch'", probe)
        self.assertNotIn("github.event_name == 'pull_request'", probe)
        self.assertIn("startsWith(github.ref, 'refs/tags/')", publish)

    def test_branch_push_is_the_gate_the_site_reports_from(self) -> None:
        """Push в main и релизную линию гоняет все тесты: отсюда сайт берёт отчёт."""
        text = self.release_text()

        self.assertIn('branches: [main, "release-v*"]', text)
        self.assertIn('    tags:\n      - "v*"', text)

    def test_release_assessment_uses_affected_mechanism_contour(self) -> None:
        text = self.release_text()
        assessment = job_block(text, "release-assessment")

        self.assertIn("needs: [classify-changes, build-tools]", assessment)
        self.assertIn("assessment_required == 'true'", assessment)
        self.assertIn("needs.build-tools.result == 'success'", assessment)

    def test_release_assessment_uses_the_candidate_release_identity(self) -> None:
        assessment = job_block(self.release_text(), "release-assessment")

        self.assertIn(
            "RELEASE_TAG: ${{ github.event_name == 'push' && "
            "startsWith(github.ref, 'refs/tags/') && github.ref_name || '' }}",
            assessment,
        )
        self.assertIn("if: ${{ env.RELEASE_TAG == '' }}", assessment)
        self.assertIn('echo "RELEASE_TAG=v${version}" >> "$GITHUB_ENV"', assessment)
        self.assertIn('--release-tag "$RELEASE_TAG"', assessment)
        self.assertNotIn("RELEASE_REF: ${{ github.ref_name }}", assessment)

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
        combined = release + publish + self.nightly_text() + self.pages_text()

        self.assertIn("actions/checkout@v7", combined)
        self.assertIn("actions/setup-python@v7", release)
        self.assertIn("actions/cache@v5", release)
        self.assertIn("actions/upload-artifact@v7", release)
        self.assertIn("actions/download-artifact@v8", release)
        self.assertIn("softprops/action-gh-release@v3", release)
        self.assertIn("github/codeql-action/upload-sarif@v4", release)
        for stale in (
            "actions/checkout@v4",
            "actions/setup-python@v5",
            "actions/cache@v4",
            "actions/upload-artifact@v4",
            "actions/download-artifact@v4",
            "softprops/action-gh-release@v2",
            "github/codeql-action/upload-sarif@v3",
        ):
            with self.subTest(stale=stale):
                self.assertNotIn(stale, combined)

    def test_heavy_and_external_jobs_have_timeouts(self) -> None:
        release = self.release_text()
        publish = self.publish_text()

        expected_release_timeouts = {
            "classify-changes": 10,
            "guards": 15,
            "test-python": 90,
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
        """Стражи реестра идут в `guards` первыми, наборы Python — в `test-python` за ними."""
        text = self.release_text()
        guards = job_block(text, "guards")
        python = job_block(text, "test-python")

        self.assertIn("python -m py_compile scripts/arch/*.py tests/arch/*.py", guards)
        self.assertIn("python scripts/arch/registry.py --check", guards)
        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem python --results', python)
        self.assertIn("needs: [classify-changes, guards]", python)

    def test_gate_profile_follows_the_event_not_the_job(self) -> None:
        """Ворота → профиль: pull request — `pr`, push в ветку — `main`, тег — `release`."""
        text = self.release_text()

        self.assertIn(
            "GATE_PROFILE: ${{ github.event_name == 'pull_request' && 'pr' || "
            "(github.event_name == 'push' && startsWith(github.ref, 'refs/tags/')) && 'release' || 'main' }}",
            text,
        )
        self.assertNotIn("run-tests.py --profile all", text)

    def test_line_rides_in_the_signature_and_tags_resolve_to_a_release_line(self) -> None:
        """Линия прогона — из resolve-line.py, в подписи результатов и плана."""
        text = self.release_text()
        classify = job_block(text, "classify-changes")

        self.assertIn('python scripts/ci/resolve-line.py --ref-type "$REF_TYPE" --ref-name "$REF_NAME" --sha "$GITHUB_SHA"', classify)
        self.assertIn("line: ${{ steps.line.outputs.line }}", classify)
        self.assertEqual(5, text.count('--line "$RUN_LINE"'))
        self.assertEqual(3, text.count("RUN_LINE: ${{ needs.classify-changes.outputs.line }}"))
        # План едет каталогом вместе с подписью, а не одним файлом.
        self.assertNotIn("path: .build/results/plan.json", text)

    def test_nightly_runs_only_moved_lines_and_signs_each_artifact_with_its_line(self) -> None:
        """Ночь: одна джоба перечисления, матрица из сдвинувшихся линий, ярус large."""
        text = self.nightly_text()
        lines = job_block(text, "lines")
        large = job_block(text, "large")

        self.assertIn("schedule:", text)
        self.assertIn("workflow_dispatch:", text)
        self.assertIn('python scripts/ci/nightly-lines.py --repo "$GITHUB_REPOSITORY" --site "$SITE"', lines)
        self.assertIn("if: needs.lines.outputs.count != '0'", large)
        self.assertIn("matrix: ${{ fromJSON(needs.lines.outputs.matrix) }}", large)
        self.assertIn("ref: ${{ matrix.sha }}", large)
        self.assertIn('--profile large --ecosystem rust --plan-only --results .build/results --runner "$RUNNER_LABEL" --line "$RUN_LINE" --sha "$RUN_SHA"', large)
        self.assertIn("name: plan-${{ matrix.line }}-rust-${{ matrix.runner }}", large)
        self.assertIn("name: results-${{ matrix.line }}-rust-${{ matrix.runner }}", large)
        self.assertIn("if: always()", large)

    def test_pages_take_results_from_red_runs_and_from_the_nightly(self) -> None:
        """Красный прогон — тоже результат; ночь и тег — тоже источники."""
        text = self.pages_text()

        self.assertIn('workflows: ["Build Unica Codex Plugin", "Unica Nightly"]', text)
        self.assertIn('branches: [main, "release-v*", "v*"]', text)
        self.assertIn("github.event.workflow_run.conclusion == 'failure'", text)
        self.assertIn("github.event.workflow_run.event == 'schedule'", text)
        self.assertIn("github.event.workflow_run.head_repository.full_name == github.repository", text)

    def test_guards_ship_findings_to_code_scanning_not_the_gate(self) -> None:
        """Находка линтера — не исход теста: SARIF в Code Scanning, гейт не краснеет."""
        guards = job_block(self.release_text(), "guards")

        self.assertIn("tool: zizmor@1.30.0", guards)
        self.assertIn("zizmor --format sarif --no-exit-codes .github/workflows > zizmor.sarif", guards)
        self.assertIn("uses: github/codeql-action/upload-sarif@v4", guards)
        self.assertIn("category: zizmor", guards)
        self.assertIn("security-events: write", guards)
        # Токен pull request из форка писать в Code Scanning не вправе.
        self.assertIn("github.event.pull_request.head.repo.full_name == github.repository", guards)

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
        probe = job_block(text, "probe-thin-bootstrap")
        smoke = job_block(text, "smoke-thin-plugin")

        expected_targets = {
            "linux-x64": "ubuntu-latest",
            "win-x64": "windows-2022",
            "darwin-arm64": "macos-14",
        }
        for target, runner in expected_targets.items():
            with self.subTest(job="probe", target=target):
                self.assertIn(f"- target: {target}", probe)
                self.assertIn(f"runner: {runner}", probe)
            with self.subTest(job="smoke", target=target):
                self.assertIn(f"- target: {target}", smoke)
                self.assertIn(f"runner: {runner}", smoke)
        self.assertEqual(probe.count("- target:"), len(expected_targets))
        self.assertEqual(smoke.count("- target:"), len(expected_targets))
        self.assertIn("Probe packaged bootstrap through the downloader", probe)
        self.assertIn("Smoke packaged bootstrap against published runtime", smoke)
        self.assertIn("scripts/ci/smoke-unica-bootstrap.py", smoke)
        self.assertIn(' --plugin-root .build/thin/plugins/unica', smoke)
        self.assertIn(' --target "${{ matrix.target }}"', smoke)
        self.assertIn("needs: package-thin", probe)
        self.assertIn("needs: [package-thin, publish-release-assets]", smoke)
        self.assertIn("--expect-download-failure", probe)

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
        # Сборка запускается и по push в main; публикацию открывает только тег.
        self.assertIn("startsWith(github.event.workflow_run.head_branch, 'v')", text)
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
        jobs = parse_workflow_jobs(self.release)
        authoritative = jobs["build-tools"].targets
        self.assertEqual(
            authoritative,
            (
                ("linux-x64", "ubuntu-latest"),
                ("win-x64", "windows-latest"),
                ("darwin-arm64", "macos-14"),
            ),
        )
        authoritative_targets = {target for target, _ in authoritative}
        for contour in ("probe-thin-bootstrap", "smoke-thin-plugin"):
            self.assertEqual(
                {target for target, _ in jobs[contour].targets},
                authoritative_targets,
                contour,
            )

        build = jobs["build-tools"]
        ordered_steps = (
            "Build target bundle and bootstrap",
            "Package deterministic runtime",
            "Verify local runtime asset pair",
            "Upload runtime metadata",
            "Upload bootstrap payload",
            "Upload required runtime archive",
        )
        positions = [build.steps.index(step) for step in ordered_steps]
        self.assertEqual(positions, sorted(positions))
        self.assertIn("tools.json", build.body)
        self.assertIn('manifest["runtimeFiles"]', build.body)
        self.assertIn("--target \"${{ matrix.target }}\"", build.body)

        expected_needs = {
            "package-thin": ("build-tools",),
            "publish-release-assets": ("build-tools",),
            "probe-thin-bootstrap": ("package-thin",),
            "smoke-thin-plugin": ("package-thin", "publish-release-assets"),
            "verify-published-assets": ("publish-release-assets", "package-thin"),
        }
        for job, needs in expected_needs.items():
            self.assertEqual(jobs[job].needs, needs, job)
            for dependency in needs:
                self.assertIn(f"needs.{dependency}.result == 'success'", jobs[job].body)

        local_verifier = "python scripts/ci/verify-release-assets.py"
        self.assertIn(local_verifier, build.body)
        self.assertIn('--asset-dir ".build/runtime-assets/${{ matrix.target }}"', build.body)
        self.assertIn('--target "${{ matrix.target }}"', build.body)

        published = jobs["verify-published-assets"]
        published_lifecycle = (
            'gh release download "$GITHUB_REF_NAME" --pattern \'unica-runtime-*\' --dir published',
            local_verifier + " --asset-dir published",
            "name: unica-thin-marketplace",
            "python scripts/ci/verify-delivery-reachable.py",
        )
        published_positions = [published.body.index(step) for step in published_lifecycle]
        self.assertEqual(published_positions, sorted(published_positions))
        self.assertNotIn("--target", published.body, "published verification must cover every target")

        smoke = jobs["smoke-thin-plugin"]
        smoke_lifecycle = (
            "Smoke packaged bootstrap against published runtime",
            "Prefetch the whole delivery once, end to end",
        )
        smoke_positions = [smoke.steps.index(step) for step in smoke_lifecycle]
        self.assertEqual(smoke_positions, sorted(smoke_positions))
        self.assertIn("matrix.target == 'linux-x64'", smoke.body)
        self.assertIn('prefetch --plugin-root .build/thin/plugins/unica', smoke.body)


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
