"""Стражи workflow: разбор структуры, а не байтов YAML.

Проверяется то, что видит GitHub: триггеры, джобы, `needs`, условия, шаги, их
`with`, `env` и скрипты. Перенос строки в условии, стиль списка или кавычки
поведения не меняют — и стража не красят. Текстом остаются две вещи:
комментарий версии у пина действия (его читает Dependabot, а не GitHub) и
маркеры, которых в файле быть не должно вовсе.
"""

from __future__ import annotations

import fnmatch
import re
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Iterator
from pathlib import Path

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import ci_rust_lanes as lanes  # noqa: E402
import prepared_binary  # noqa: E402


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
RELEASE_WORKFLOW = WORKFLOWS_DIR / "unica-plugin-release.yml"
NIGHTLY_WORKFLOW = WORKFLOWS_DIR / "unica-nightly.yml"
PAGES_WORKFLOW = WORKFLOWS_DIR / "unica-pages.yml"
PUBLISH_WORKFLOW = WORKFLOWS_DIR / "publish-unica-marketplace.yml"
LEGACY_WORKFLOW = WORKFLOWS_DIR / "unica-legacy-migration.yml"


def load(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def triggers(workflow: dict) -> dict:
    """`on:` — PyYAML читает ключ по YAML 1.1 как булево `True`."""
    value = workflow.get("on") or workflow.get(True) or {}
    if isinstance(value, str):
        return {value: None}
    if isinstance(value, list):
        return dict.fromkeys(value)
    return value


def metadata_build_triggers(workflow: dict) -> dict[str, list[str]]:
    """Прямые события назначения меток и вех; условия джоб не отменяют suite."""
    metadata = {"labeled", "unlabeled", "milestoned", "demilestoned"}
    found = {}
    for event, config in triggers(workflow).items():
        if event not in {"pull_request", "pull_request_target", "issues"}:
            continue
        # Без types оба PR-события принимают только opened/synchronize/reopened,
        # а issues принимает все действия, включая назначение меток и вех.
        defaults = metadata if event == "issues" else set()
        activities = (config or {}).get("types", defaults)
        if isinstance(activities, str):
            activities = [activities]
        matched = sorted(metadata.intersection(activities))
        if matched:
            found[event] = matched
    return found


def jobs(workflow: dict) -> dict:
    return workflow["jobs"]


def job(workflow: dict, job_id: str) -> dict:
    found = jobs(workflow).get(job_id)
    assert found is not None, f"в workflow нет джобы {job_id}"
    return found


def needs(node: dict) -> list[str]:
    value = node.get("needs", [])
    return [value] if isinstance(value, str) else list(value)


def steps(node: dict) -> list[dict]:
    return list(node.get("steps", []))


def scripts(node: dict) -> list[str]:
    return [step["run"] for step in steps(node) if "run" in step]


def script(node: dict) -> str:
    return "\n".join(scripts(node))


def all_scripts(workflow: dict) -> str:
    return "\n".join(script(found) for found in jobs(workflow).values())


def normalized(value: object) -> str:
    """Выражение без переносов и лишних пробелов: перенос строки — не смысл."""
    return " ".join(str(value).split())


def condition(node: dict) -> str:
    return normalized(node.get("if", ""))


def expressions(node: dict) -> list[str]:
    """Всё, что вычисляет GitHub в джобе: условия, `env` и `with` джобы и шагов."""
    found = [condition(node), *(normalized(value) for value in (node.get("env") or {}).values())]
    for step in steps(node):
        found.append(condition(step))
        found.extend(normalized(value) for value in (step.get("env") or {}).values())
        found.extend(normalized(value) for value in (step.get("with") or {}).values())
    return found


def strings(node: object) -> Iterator[str]:
    """Каждый скаляр и ключ дерева — для маркеров, которых быть не должно."""
    if isinstance(node, dict):
        for key, value in node.items():
            yield str(key)
            yield from strings(value)
    elif isinstance(node, list):
        for item in node:
            yield from strings(item)
    elif node is not None:
        yield str(node)


def action(step: dict) -> str:
    return step.get("uses", "").split("@", 1)[0]


def steps_using(node: dict, prefix: str) -> list[dict]:
    return [step for step in steps(node) if action(step) == prefix]


def step_named(node: dict, name: str) -> dict:
    for step in steps(node):
        if step.get("name") == name:
            return step
    raise AssertionError(f"нет шага {name!r}")


def step_by_id(node: dict, step_id: str) -> dict:
    for step in steps(node):
        if step.get("id") == step_id:
            return step
    raise AssertionError(f"нет шага с id {step_id!r}")


def step_index(node: dict, name: str) -> int:
    return steps(node).index(step_named(node, name))


def step_text(step: dict) -> str:
    """Шаг одной строкой: имя, действие, `with` и скрипт — для порядка шагов."""
    return "\n".join([step.get("name", ""), step.get("uses", ""), *map(str, (step.get("with") or {}).values()), step.get("run", "")])


def uploads(node: dict) -> list[dict]:
    return [step.get("with") or {} for step in steps_using(node, "actions/upload-artifact")]


def downloads(node: dict) -> list[dict]:
    return [step.get("with") or {} for step in steps_using(node, "actions/download-artifact")]


def matrix_include(node: dict) -> list[dict]:
    return list((node.get("strategy") or {}).get("matrix", {}).get("include", []))


def targets(node: dict) -> list[tuple[str, str]]:
    return [(entry["target"], entry["runner"]) for entry in matrix_include(node)]


def pinned(action_name: str, major: str) -> str:
    """Действие закреплено хешем коммита, а версия названа комментарием рядом.

    Dependabot двигает хеш вместе с комментарием, поэтому тест держит только
    мажорную версию: минорный сдвиг проходит, смена мажора требует правки.
    Комментарий не видит ни GitHub, ни разбор YAML — это единственная проверка
    по тексту файла.
    """
    return rf"uses: {re.escape(action_name)}@[0-9a-f]{{40}} # {re.escape(major)}(\.\d+)*\b"


def assert_pinned(test: unittest.TestCase, path: Path, step: dict, major: str) -> None:
    """Шаг закреплён хешем, и комментарий рядом с этим самым хешем называет мажор."""
    action_name, _, ref = step.get("uses", "").partition("@")
    test.assertRegex(ref, r"^[0-9a-f]{40}$", step.get("uses"))
    test.assertRegex(
        path.read_text(encoding="utf-8"),
        rf"uses: {re.escape(action_name)}@{ref} # {re.escape(major)}(\.\d+)*\b",
    )


class MetadataBuildTriggerTests(unittest.TestCase):
    def test_code_and_manual_events_do_not_match_metadata_changes(self) -> None:
        for source in (
            "on: pull_request",
            "on: [pull_request_target, push, workflow_dispatch]",
            "'on': {pull_request: null, pull_request_target: {branches: [main]}}",
            """on:
  pull_request:
    types: [opened, synchronize, reopened]
  merge_group:
    types: [checks_requested]
  push:
    branches: [main, release-v*]
    tags: [v*]
  workflow_dispatch:
""",
            "on: {issues: {types: [opened, closed]}, issue_comment: null}",
        ):
            with self.subTest(source=source):
                self.assertEqual(metadata_build_triggers(yaml.safe_load(source)), {})

    def test_metadata_changes_are_found_through_each_direct_event(self) -> None:
        for event in ("pull_request", "pull_request_target", "issues"):
            for activity in ("labeled", "unlabeled", "milestoned", "demilestoned"):
                for types in (activity, f"[opened, {activity}]"):
                    with self.subTest(event=event, types=types):
                        source = f"on: {{{event}: {{types: {types}}}}}"
                        self.assertEqual(
                            metadata_build_triggers(yaml.safe_load(source)),
                            {event: [activity]},
                        )

    def test_issues_shorthand_includes_label_and_milestone_changes(self) -> None:
        for source in ("on: issues", "on: [push, issues]", "on: {issues: null}", "on: {issues: {}}"):
            with self.subTest(source=source):
                self.assertEqual(
                    metadata_build_triggers(yaml.safe_load(source)),
                    {"issues": ["demilestoned", "labeled", "milestoned", "unlabeled"]},
                )


class UnicaWorkflowGuardrailTests(unittest.TestCase):
    def setUp(self) -> None:
        self.release = load(RELEASE_WORKFLOW)
        self.nightly = load(NIGHTLY_WORKFLOW)
        self.pages = load(PAGES_WORKFLOW)
        self.publish = load(PUBLISH_WORKFLOW)

    def test_source_gate_checks_the_full_rust_and_python_workspace(self) -> None:
        text = all_scripts(self.release)

        self.assertIn("cargo clippy --workspace --all-targets --all-features --message-format=json -- -D warnings", text)
        # Наборы гоняет шов; сами команды закреплены тестом `test_run_tests`.
        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem rust --rust-lane ordinary --phase run', text)
        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem python --suite "$SUITE" --only-size "$LANE" --docs-only "$DOCS_ONLY" --results', text)
        # Исполнение тестов остаётся у runner. Прогрев имеет отдельный
        # контракт compile-only, проверенный ниже для каждого его шага.
        for job_id, found in jobs(self.release).items():
            if job_id != "warm-rust-cache":
                self.assertNotIn("cargo test", script(found))
        self.assertNotIn("unittest discover", text)
        self.assertIn("python -m py_compile scripts/dev/*.py tests/dev/*.py", text)
        self.assertIn("python scripts/ci/check-version-contract.py", text)

    def test_label_changes_cannot_replace_the_stable_pull_request_gate(self) -> None:
        on = triggers(self.release)
        gate = job(self.release, "unica-ci")

        # Изменение метки не должно создавать новый suite этого workflow:
        # ruleset смотрит на последний suite головы PR и перестаёт видеть
        # обязательный `Unica CI`, если тот suite состоит из skipped-джоб.
        self.assertEqual(on["pull_request"]["types"], ["opened", "synchronize", "reopened"])
        self.assertNotIn("paths", on["pull_request"])
        self.assertNotIn("paths", on["push"])
        self.assertNotIn("label ignored", "\n".join(strings(self.release)))
        self.assertEqual(gate["name"], "Unica CI")
        self.assertEqual(condition(gate), "always()")
        self.assertEqual(condition(job(self.release, "classify-changes")), "")
        self.assertEqual(condition(job(self.release, "guards")), "")
        self.assertEqual(
            normalized(self.release["concurrency"]["group"]),
            "${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}"
            "${{ github.event_name == 'workflow_dispatch' && format('-dispatch-{0}', inputs.profile) || '' }}",
        )
        self.assertIn("python scripts/ci/evaluate-ci-gate.py", script(gate))
        for upstream in (
            "classify-changes",
            "guards",
            "test-python",
            "test-rust-platforms",
            "lint-rust-platforms",
            "warm-rust-cache",
            "build-tools",
            "package-thin",
            "probe-thin-bootstrap",
            "release-assessment",
            "publish-release-assets",
            "smoke-thin-plugin",
            "verify-published-assets",
        ):
            with self.subTest(upstream=upstream):
                self.assertIn(upstream, needs(gate))

    def test_build_workflow_does_not_subscribe_to_label_or_milestone_changes(self) -> None:
        # Проверяем все входы сборочного workflow: безопасный pull_request
        # не исключает параллельный pull_request_target с metadata types.
        self.assertEqual(metadata_build_triggers(self.release), {})

    def test_p0_dry_release_proof_is_read_only_and_aggregated(self) -> None:
        proof = job(self.release, "p0-release-proof")

        self.assertEqual(set(needs(proof)), {"build-tools", "package-thin", "release-assessment"})
        for argument in (
            "scripts/ci/release-proof.py",
            "--surface-ledger docs/tool-surface.md",
            "--wire-dir",
            "--package-dir",
            "--asset-verification-dir",
            "--source-commit",
            "--baseline",
            "--out-dir dist/p0-proof",
        ):
            self.assertIn(argument, script(proof))
        # Единственный режим proof — dry: сценарии жизненного цикла доказываются
        # на опубликованных байтах, а эта джоба идёт до публикации (#697).
        self.assertNotIn("--mode", script(proof))
        # Идентичность пакета включает бит исполнения (#700), а
        # actions/download-artifact режимы не сохраняет: пакет для сверки
        # приходит через `gh run download`, которому нужно `actions: read`.
        self.assertEqual(proof["permissions"], {"contents": "read", "actions": "read"})
        self.assertIn("gh run download", script(proof))
        self.assertIn("unica-thin-marketplace", script(proof))
        for step in steps_using(proof, "actions/download-artifact"):
            self.assertNotEqual((step.get("with") or {}).get("name"), "unica-thin-marketplace")
        self.assertEqual(steps_using(proof, "softprops/action-gh-release"), [])
        # Read-only-ность proof живёт здесь, в устройстве джобы, а не в
        # самоотчётных полях JSON (#696): нет прав на запись, нет шагов
        # публикации, нет ни тега, ни push, ни бампа версии.
        self.assertNotIn("git tag", script(proof))
        self.assertNotIn("git push", script(proof))
        self.assertNotIn("gh release", script(proof))
        self.assertNotIn("bump-version", script(proof))
        self.assertIn("p0-release-proof", needs(job(self.release, "unica-ci")))

    def test_wire_probes_embed_the_matrix_target_in_their_evidence(self) -> None:
        build = job(self.release, "build-tools")
        for name in ("Probe native package wire profile", "Probe compatibility package wire profile"):
            with self.subTest(step=name):
                self.assertIn('--target "$TARGET"', step_text(step_named(build, name)))

    def test_classifier_exposes_typed_contours_and_event_routing(self) -> None:
        classifier = job(self.release, "classify-changes")

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
            "docs_only",
            "rust_required",
            "cache_required",
        ):
            with self.subTest(output=output):
                self.assertEqual(classifier["outputs"][output], "${{ steps.scope.outputs." + output + " }}")
        scope = step_by_id(classifier, "scope")
        # Payload события, включая ci:full и неизменяемые SHA, читает Python.
        # Workflow передаёт только уже выбранную матрицу и линию отчёта.
        self.assertEqual(scope["env"], {
            "RUNNERS": "${{ steps.runners.outputs.runners }}",
            "RUN_LINE": "${{ steps.line.outputs.line }}",
        })
        self.assertEqual(
            normalized(scope["run"]),
            'python scripts/ci/classify-workflow-changes.py --github-event --runner-matrix "$RUNNERS" --line "$RUN_LINE" --results .build/ci-routing >> "$GITHUB_OUTPUT"',
        )
        routing = steps_using(classifier, "actions/upload-artifact")
        self.assertEqual(len(routing), 1)
        self.assertEqual(condition(routing[0]), "steps.scope.outputs.docs_only == 'true'")
        self.assertEqual(routing[0]["with"], {
            "name": "results-ci-routing", "path": ".build/ci-routing", "retention-days": 7,
            "if-no-files-found": "error",
        })

    def test_classifier_keeps_history_and_does_not_fetch_a_mutable_base(self) -> None:
        classifier = job(self.release, "classify-changes")
        checkout = steps_using(classifier, "actions/checkout")[0]
        scope = step_by_id(classifier, "scope")

        self.assertEqual(checkout["with"]["fetch-depth"], 0)
        self.assertNotIn("BASE_REF", scope.get("env", {}))
        self.assertNotIn("FORCE_FULL", scope.get("env", {}))
        self.assertNotIn("git fetch", scope["run"])
        self.assertNotIn("FETCH_HEAD", scope["run"])
        self.assertIn("--github-event", scope["run"])

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
        for workflow in sorted((*WORKFLOWS_DIR.glob("*.yml"), *WORKFLOWS_DIR.glob("*.yaml")), key=lambda path: path.name):
            for job_id, found in jobs(load(workflow)).items():
                for index, step in enumerate(steps(found)):
                    if "run" not in step:
                        continue
                    scanned += sum(1 for line in step["run"].splitlines() if line.strip())
                    # Скрипт видит только то, что GitHub подставил: `${{ }}` в
                    # `run:` — единственное место, где данные становятся кодом.
                    for expression in re.findall(r"\$\{\{(.*?)\}\}", step["run"], re.S):
                        context = next((ref for ref in refs if ref in expression), None)
                        with self.subTest(workflow=workflow.name, job=job_id, step=index):
                            self.assertIsNone(
                                context,
                                f"{context} is interpolated into a run block; bind it "
                                'to an env variable and read it as "$NAME"',
                            )

        # A scanner that silently matched nothing would pass this test forever.
        # The workflows have far more shell than this.
        self.assertGreater(scanned, 50)

    def test_rust_jobs_run_the_full_matrix_for_any_rust_change(self) -> None:
        classify = job(self.release, "classify-changes")
        lint = job(self.release, "lint-rust-platforms")
        aggregate = job(self.release, "test-rust-platforms")
        self.assertEqual(lint["strategy"]["matrix"]["runner"], "${{ fromJSON(needs.classify-changes.outputs.runners) }}")
        self.assertEqual(condition(lint), "${{ needs.classify-changes.outputs.rust_required == 'true' }}")
        self.assertIn("always()", condition(aggregate))
        self.assertIn("--phase verify", script(aggregate))
        self.assertIn("CI_LANE_JOB_RESULTS", aggregate["env"])
        for lane in ("ordinary", "receipt", "vendor"):
            producer, executor = job(self.release, "build-rust-" + lane), job(self.release, "run-rust-" + lane)
            self.assertEqual(needs(producer), ["classify-changes"])
            self.assertIn("build-rust-" + lane, needs(executor))
            for other in ("ordinary", "receipt", "vendor"):
                if other != lane:
                    self.assertNotIn("build-rust-" + other, needs(executor))
            for phase, found in (("build", producer), ("run", executor)):
                self.assertEqual(found["strategy"]["matrix"]["include"], "${{ fromJSON(needs.classify-changes.outputs." + lane + "_" + phase + "_matrix) }}")
                self.assertIn("rust_required == 'true'", condition(found))
                self.assertIn("--phase " + phase, script(found))
                self.assertIn("--rust-lane " + lane, script(found))
                self.assertIn(phase + "-rust-" + lane, needs(aggregate))
            self.assertTrue(any(upload["name"].startswith("plan-rust-") for upload in uploads(producer)))
            self.assertTrue(any(download["name"].startswith("ci-rust-archive-") for download in downloads(executor) if "name" in download))
            self.assertFalse(steps_using(executor, "Swatinem/rust-cache"), "archive execution receives no Cargo cache")
            if lane != "ordinary":
                self.assertIn("github.event_name != 'pull_request'", condition(producer))
        self.assertNotIn("cargo fmt", script(aggregate))
        self.assertIn("cargo fmt --all -- --check", script(job(self.release, "guards")))
        sarif = steps_using(lint, "github/codeql-action/upload-sarif")[0]
        self.assertEqual(sarif["with"]["category"], "clippy-${{ matrix.runner }}")
        self.assertIn("always()", condition(sarif))
        self.assertIn("hashFiles('clippy.sarif') != ''", condition(sarif))
        self.assertEqual(lint["permissions"], {"contents": "read", "security-events": "write"})
        clippy = next(step for step in steps(lint) if "cargo clippy" in step.get("run", ""))
        self.assertNotIn("continue-on-error", clippy)
        self.assertIn("--all-targets --all-features --message-format=json -- -D warnings", clippy["run"])

    def test_package_contour_and_pr_smoke_do_not_publish_release_assets(self) -> None:
        build = condition(job(self.release, "build-tools"))
        probe = condition(job(self.release, "probe-thin-bootstrap"))
        publish = condition(job(self.release, "publish-release-assets"))

        self.assertIn("needs.classify-changes.outputs.release_required == 'true'", build)
        self.assertIn("needs.classify-changes.outputs.ci_changed == 'true'", build)
        # Сборка и холодный старт сняты с pull request и с push в ветку: там они
        # не давали прослеживаемости, а гейт красили. Тег и ручной запуск их
        # сохраняют.
        self.assertIn("(github.event_name == 'workflow_dispatch' || startsWith(github.ref, 'refs/tags/'))", build)
        self.assertNotIn("github.event_name != 'pull_request'", build)
        self.assertIn("github.event_name == 'workflow_dispatch'", probe)
        self.assertNotIn("github.event_name == 'pull_request'", probe)
        self.assertIn("startsWith(github.ref, 'refs/tags/')", publish)

    def test_release_line_push_is_the_gate_the_site_reports_from(self) -> None:
        """Push в релизную линию гоняет все тесты: очереди там нет, отсюда сайт берёт отчёт линии."""
        push = triggers(self.release)["push"]

        self.assertEqual(push["branches"], ["main", "release-v*"])
        self.assertEqual(push["tags"], ["v*"])

    def test_main_push_after_the_queue_only_refreshes_the_dependency_cache(self) -> None:
        main_push = "(github.event_name == 'push' && github.ref == 'refs/heads/main')"
        python = job(self.release, "test-python")
        self.assertIn("!" + main_push, condition(python))
        self.assertIn("!" + main_push, condition(job(self.release, "build-python-product")))
        self.assertIn("needs.build-python-product.result == 'success'", condition(python))
        self.assertIn("needs.build-python-product.result == 'skipped'", condition(python))
        self.assertNotIn("python_matrix == '[]'", condition(python))
        self.assertIn("rust_required == 'true'", condition(job(self.release, "test-rust-platforms")))
        warm = job(self.release, "warm-rust-cache")
        self.assertEqual(condition(warm), "${{ needs.classify-changes.outputs.cache_required == 'true' }}")
        self.assertNotIn("run-tests.py", script(warm))
        self.assertEqual(job(self.release, "classify-changes")["outputs"]["cache_required"], "${{ steps.scope.outputs.cache_required }}")

    def test_cache_warm_compiles_without_executing_validation(self) -> None:
        warm = job(self.release, "warm-rust-cache")

        self.assertEqual(condition(warm), "${{ needs.classify-changes.outputs.cache_required == 'true' }}")
        self.assertEqual(needs(warm), ["classify-changes"])
        self.assertEqual(warm["strategy"]["matrix"]["runner"], "${{ fromJSON(needs.classify-changes.outputs.runners) }}")
        compile_steps = [step for step in steps(warm) if "run" in step]
        self.assertEqual(
            [normalized(step["run"]) for step in compile_steps],
            [
                "cargo test --workspace --no-run --locked",
                "cargo test -p unica-coder --features receipt-ledger-test-support --test daemon_receipt_ledger --no-run --locked",
                "cargo check --workspace --all-targets --all-features --locked",
            ],
        )
        for step in compile_steps:
            self.assertEqual(condition(step), "steps.rust-cache.outputs.cache-hit != 'true'")
        self.assertNotIn("clippy", script(warm))
        self.assertNotIn("nextest", script(warm))
        self.assertNotIn("run-tests.py", script(warm))
        self.assertEqual(uploads(warm), [])
        self.assertEqual(steps_using(warm, "taiki-e/install-action"), [])

    def test_lint_and_test_failures_cannot_cancel_other_validation_jobs(self) -> None:
        for job_id in ("test-python", "lint-rust-platforms", "build-rust-ordinary", "build-rust-receipt", "build-rust-vendor", "run-rust-ordinary", "run-rust-receipt", "run-rust-vendor"):
            with self.subTest(job=job_id):
                found = job(self.release, job_id)
                self.assertNotIn("guards", needs(found))
                self.assertNotIn("lint-rust-platforms", needs(found))
                self.assertIs(found["strategy"]["fail-fast"], False)
        rust, lint = job(self.release, "test-rust-platforms"), job(self.release, "lint-rust-platforms")
        self.assertNotIn("cargo clippy", script(rust))
        self.assertEqual(steps_using(rust, "github/codeql-action/upload-sarif"), [])
        self.assertNotIn("run-tests.py", script(lint))
        self.assertNotIn("security-events", rust.get("permissions", {}))
        build = job(self.release, "build-tools")
        self.assertIn("guards", needs(build))
        self.assertIn("lint-rust-platforms", needs(build))
        self.assertIn("needs.lint-rust-platforms.result == 'success'", condition(build))
        for upstream in ("guards", "lint-rust-platforms", "warm-rust-cache", "test-python", "test-rust-platforms", "build-python-product"):
            self.assertIn(upstream, needs(job(self.release, "unica-ci")))

    def test_rust_cache_is_shared_without_a_lint_only_writer(self) -> None:
        for job_id in ("lint-rust-platforms", "warm-rust-cache"):
            found = job(self.release, job_id)
            cache = steps_using(found, "Swatinem/rust-cache")
            self.assertEqual(len(cache), 1)
            assert_pinned(self, RELEASE_WORKFLOW, cache[0], "v2")
            self.assertEqual(cache[0]["with"]["shared-key"], "test-rust-platforms")
            self.assertEqual(found["env"]["CARGO_PROFILE_DEV_DEBUG"], "line-tables-only")
        lint = job(self.release, "lint-rust-platforms")
        self.assertIs(steps_using(lint, "Swatinem/rust-cache")[0]["with"]["save-if"], False)
        self.assertEqual(normalized(lint["continue-on-error"]), "${{ matrix.runner == 'windows-latest' }}")
        self.assertEqual(steps_using(lint, "taiki-e/install-action")[0]["with"]["tool"], "clippy-sarif@0.8.0,sarif-fmt@0.8.0")
        # Признаки, объединённые workspace, прячут мёртвый код отдельного пакета.
        per_package = next(step for step in steps(lint) if "cargo clippy -p" in step.get("run", ""))
        self.assertEqual(normalized(per_package["if"]), "matrix.runner == 'ubuntu-latest'")
        self.assertIn("cargo metadata --no-deps", per_package["run"])
        # Отказ подстановки в заголовке `for` даёт ноль итераций и зелёный шаг.
        self.assertNotRegex(per_package["run"], r"for \w+ in \$\(")
        self.assertIn('"${#packages[@]}" -eq 0', per_package["run"])
        self.assertIn("--all-targets --locked -- -D warnings", per_package["run"])
        self.assertNotIn("--all-features", per_package["run"])
        self.assertNotIn("--workspace", per_package["run"])
        warm = step_by_id(job(self.release, "warm-rust-cache"), "rust-cache")
        self.assertIs(warm["with"]["save-if"], True)
        self.assertIs(warm["with"]["cache-on-failure"], False)
        for lane in ("ordinary", "receipt", "vendor"):
            producer, executor = job(self.release, "build-rust-" + lane), job(self.release, "run-rust-" + lane)
            cache = steps_using(producer, "Swatinem/rust-cache")[0]
            self.assertEqual(cache["with"]["shared-key"], "ci-rust-" + lane)
            # Producer восстанавливает только зависимости: крейты workspace
            # rust-cache вычищает перед записью и не включает их обратно.
            self.assertNotIn("lookup-only", cache["with"])
            self.assertNotIn("cache-workspace-crates", cache["with"])
            # Кешируется тот каталог, куда собирает архив: `<root>/target`.
            self.assertNotIn("workspaces", cache["with"])
            archive = lanes.archive_command(lane, Path("tests.tar.zst"))
            self.assertEqual(Path(archive[archive.index("--target-dir") + 1]), lanes.ROOT / "target")
            # nextest ставится до восстановления: rust-cache исключает уже
            # лежащие в ~/.cargo/bin файлы из записи и не подменяет их.
            order = producer["steps"]
            self.assertLess(order.index(steps_using(producer, "taiki-e/install-action")[0]), order.index(cache))
            self.assertEqual(producer["env"]["CARGO_PROFILE_DEV_DEBUG"], executor["env"]["CARGO_PROFILE_DEV_DEBUG"])
            for found in (producer, executor):
                self.assertEqual(steps_using(found, "taiki-e/install-action")[0]["with"]["tool"], "cargo-nextest@0.9.148")
        self.assertFalse(steps_using(job(self.release, "test-rust-platforms"), "Swatinem/rust-cache"))
        product_cache = steps_using(job(self.release, "build-python-product"), "Swatinem/rust-cache")
        self.assertEqual(len(product_cache), 1)
        self.assertNotIn("lookup-only", product_cache[0]["with"])
        self.assertNotIn("cache-workspace-crates", product_cache[0]["with"])
        # Продукт собирается в собственный target-dir: кешироваться должен он.
        self.assertEqual(product_cache[0]["with"]["workspaces"], f". -> {prepared_binary.PRODUCT_TARGET.as_posix()}")

    def test_release_assessment_uses_affected_mechanism_contour(self) -> None:
        assessment = job(self.release, "release-assessment")

        self.assertEqual(needs(assessment), ["classify-changes", "build-tools"])
        self.assertIn("needs.classify-changes.outputs.assessment_required == 'true'", condition(assessment))
        self.assertIn("needs.build-tools.result == 'success'", condition(assessment))

    def test_release_assessment_uses_the_candidate_release_identity(self) -> None:
        assessment = job(self.release, "release-assessment")
        resolve = step_named(assessment, "Resolve the release tag for non-tag builds")

        self.assertEqual(
            normalized(assessment["env"]["RELEASE_TAG"]),
            "${{ github.event_name == 'push' && startsWith(github.ref, 'refs/tags/') && github.ref_name || '' }}",
        )
        self.assertEqual(condition(resolve), "${{ env.RELEASE_TAG == '' }}")
        self.assertIn('echo "RELEASE_TAG=v${version}" >> "$GITHUB_ENV"', resolve["run"])
        self.assertIn('--release-tag "$RELEASE_TAG"', script(assessment))
        self.assertNotIn("RELEASE_REF", assessment["env"])

    def test_only_tag_pushes_enable_release_behavior(self) -> None:
        for job_id in ("build-tools", "package-thin"):
            with self.subTest(job_id=job_id):
                self.assertTrue(
                    any(
                        "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/')" in expression
                        for expression in expressions(job(self.release, job_id))
                    ),
                    f"{job_id}: тег отличается от push в ветку только в этом выражении",
                )
        for job_id in ("publish-release-assets", "smoke-thin-plugin", "verify-published-assets"):
            with self.subTest(job_id=job_id):
                gate = condition(job(self.release, job_id))
                self.assertIn("github.event_name == 'push'", gate)
                self.assertIn("startsWith(github.ref, 'refs/tags/')", gate)

    def test_conditional_pipeline_breaks_transitive_skip_propagation(self) -> None:
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
                gate = condition(job(self.release, job_id))
                self.assertIn("always()", gate)
                for dependency_result in dependency_results:
                    self.assertIn(dependency_result, gate)

    def test_javascript_actions_use_node24_compatible_majors(self) -> None:
        release = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        combined = "".join(path.read_text(encoding="utf-8") for path in (RELEASE_WORKFLOW, PUBLISH_WORKFLOW, NIGHTLY_WORKFLOW, PAGES_WORKFLOW))

        self.assertRegex(combined, pinned("actions/checkout", "v7"))
        self.assertRegex(release, pinned("actions/setup-python", "v7"))
        self.assertRegex(release, pinned("actions/cache", "v6"))
        self.assertRegex(release, pinned("actions/upload-artifact", "v7"))
        self.assertRegex(release, pinned("actions/download-artifact", "v8"))
        self.assertRegex(release, pinned("softprops/action-gh-release", "v3"))
        self.assertRegex(release, pinned("github/codeql-action/upload-sarif", "v4"))
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
        expected_release_timeouts = {
            "classify-changes": 10,
            "guards": 15,
            "test-python": 90,
            # Час всем воротам, три — ночному `large`: там Windows гоняет весь
            # набор вместе с контрактом ReceiptLedger.
            "test-rust-platforms": 60,
            "lint-rust-platforms": 60,
            "warm-rust-cache": 60,
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
                self.assertEqual(job(self.release, job_id).get("timeout-minutes"), minutes)

        expected_publish_timeouts = {
            "stage": 20,
            "anchor": 10,
            "verify-fresh-install": 30,
            "verify-upgrade": 30,
            "promote": 10,
        }
        for job_id, minutes in expected_publish_timeouts.items():
            with self.subTest(job_id=job_id):
                self.assertEqual(job(self.publish, job_id).get("timeout-minutes"), minutes)

    def test_python_suites_use_the_runner_matrix_without_waiting_for_guards(self) -> None:
        """Наборы Python используют матрицу штатного runner независимо от guards."""
        python = job(self.release, "test-python")

        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem python --suite "$SUITE" --only-size "$LANE" --docs-only "$DOCS_ONLY" --results', script(python))
        self.assertEqual(needs(python), ["classify-changes", "build-python-product"])
        # Матрицу джоб Python считает шов по воротам; джоба знает набор и полосу.
        self.assertEqual(python["strategy"]["matrix"]["include"], "${{ fromJSON(needs.classify-changes.outputs.python_matrix) }}")
        self.assertEqual(python["env"]["SUITE"], "${{ matrix.suite }}")
        self.assertEqual(python["env"]["LANE"], "${{ matrix.lane }}")
        self.assertEqual(python["env"]["DOCS_ONLY"], "${{ needs.classify-changes.outputs.docs_only }}")
        self.assertEqual(python["name"], "Python tests (${{ matrix.slug }})")
        classify = job(self.release, "classify-changes")
        self.assertEqual(classify["outputs"]["python_matrix"], "${{ steps.python-matrix.outputs.python_matrix }}")
        self.assertIn('run-tests.py --profile "$GATE_PROFILE" --python-matrix', step_by_id(classify, "python-matrix")["run"])
        self.assertEqual(step_by_id(classify, "python-matrix")["env"]["DOCS_ONLY"], "${{ steps.scope.outputs.docs_only }}")
        ids = [step.get("id") for step in steps(classify)]
        self.assertLess(ids.index("scope"), ids.index("python-matrix"))
        for run_line in [step["run"] for step in steps(python) if "run-tests.py" in step.get("run", "")]:
            self.assertIn('--suite "$SUITE" --only-size "$LANE"', run_line)
            self.assertIn('--docs-only "$DOCS_ONLY"', run_line)
        self.assertEqual([upload["name"] for upload in uploads(python)], ["plan-python-${{ matrix.slug }}", "results-python-${{ matrix.slug }}"])
        # План выгружается до тестов, как у Rust.
        plan_index = next(index for index, step in enumerate(steps(python)) if "--plan-only" in step.get("run", ""))
        run_index = next(index for index, step in enumerate(steps(python)) if "run-tests.py" in step.get("run", "") and "--plan-only" not in step["run"])
        self.assertLess(plan_index, run_index)

    def test_agent_evaluation_and_credential_are_absent_from_every_workflow(self) -> None:
        for workflow in sorted((*WORKFLOWS_DIR.glob("*.yml"), *WORKFLOWS_DIR.glob("*.yaml"))):
            values = list(strings(load(workflow)))
            with self.subTest(workflow=workflow.name):
                self.assertFalse(any("tests/agent_evaluation" in value for value in values))
                self.assertFalse(any("CODEX_API_KEY" in value for value in values))

    def test_gate_profile_follows_the_event_not_the_job(self) -> None:
        """Ворота → профиль: pull request — `pr`, push в ветку — `main`, тег — `release`."""
        self.assertEqual(
            normalized(self.release["env"]["GATE_PROFILE"]),
            "${{ github.event_name == 'pull_request' && 'pr' || github.event_name == 'merge_group' && 'queue' || "
            "github.event_name == 'workflow_dispatch' && inputs.profile || "
            "(github.event_name == 'push' && startsWith(github.ref, 'refs/tags/')) && 'release' || 'main' }}",
        )
        # Очередь слияния: конвейер отвечает на merge_group, иначе очередь ждёт вечно.
        self.assertEqual(triggers(self.release)["merge_group"], {"types": ["checks_requested"]})
        self.assertNotIn("run-tests.py --profile all", all_scripts(self.release))

    def test_line_rides_in_the_signature_and_tags_resolve_to_a_release_line(self) -> None:
        classify = job(self.release, "classify-changes")
        self.assertIn('python scripts/ci/resolve-line.py --ref-type "$REF_TYPE" --ref-name "$REF_NAME" --sha "$GITHUB_SHA"', script(classify))
        self.assertEqual(classify["outputs"]["line"], "${{ steps.line.outputs.line }}")
        self.assertEqual(2, script(job(self.release, "test-python")).count('--line "$RUN_LINE"'))
        expected = ["test-python"]
        for lane in ("ordinary", "receipt", "vendor"):
            for phase in ("build", "run"):
                key = phase + "-rust-" + lane
                expected.append(key)
                self.assertEqual(1, script(job(self.release, key)).count('--line "$RUN_LINE"'))
        signed = [key for key, found in jobs(self.release).items() if (found.get("env") or {}).get("RUN_LINE") == "${{ needs.classify-changes.outputs.line }}"]
        self.assertEqual(sorted(expected), sorted(signed))
        for found in jobs(self.release).values():
            for upload in uploads(found):
                self.assertNotEqual(upload.get("path"), ".build/results/plan.json")

    def test_nightly_dispatches_the_manual_contour_with_the_large_profile(self) -> None:
        """Ночь перечисляет и запускает ручной контур сборки на линии; Windows — только там."""
        lines = job(self.nightly, "lines")
        classify = job(self.release, "classify-changes")
        platforms = job(self.release, "build-rust-ordinary")

        self.assertIn("schedule", triggers(self.nightly))
        # Круглые минуты GitHub исполняет с наибольшим опозданием.
        minute = triggers(self.nightly)["schedule"][0]["cron"].split()[0]
        self.assertNotIn(minute, ("0", "30"))
        # Ручной и ночной запуск идут своей группой: push их не вытесняет.
        self.assertIn("format('-dispatch-{0}', inputs.profile)", normalized(self.release["concurrency"]["group"]))
        self.assertEqual(lines["permissions"].get("actions"), "write")
        self.assertIn("--dispatch --follow .build/large", script(lines))
        self.assertEqual([upload["name"] for upload in uploads(lines)], ["results-nightly"])
        for step in steps_using(lines, "actions/checkout"):
            self.assertNotIn("ref", step.get("with") or {})
        self.assertEqual(triggers(self.release)["workflow_dispatch"]["inputs"]["profile"]["options"], ["main", "large"])
        self.assertIn("github.event_name == 'workflow_dispatch' && inputs.profile", normalized(self.release["env"]["GATE_PROFILE"]))
        self.assertEqual(classify["outputs"]["runners"], "${{ steps.runners.outputs.runners }}")
        self.assertIn('runners=["ubuntu-latest", "macos-14", "windows-latest"]', script(classify))
        # Две полосы: pull request — ubuntu, полная матрица — в очереди, на push
        # и по кнопке; ci:full возвращает её на следующем прогоне pull request.
        runners = step_by_id(classify, "runners")
        self.assertIn('runners=["ubuntu-latest"]', runners["run"])
        self.assertIn('runners=["ubuntu-latest", "macos-14"]', runners["run"])
        self.assertEqual(
            normalized(runners["env"]["FULL_MATRIX"]),
            "${{ github.event_name != 'pull_request' || contains(github.event.pull_request.labels.*.name, 'ci:full') }}",
        )
        self.assertEqual(platforms["strategy"]["matrix"]["include"], "${{ fromJSON(needs.classify-changes.outputs.ordinary_build_matrix) }}")
        self.assertEqual(platforms["defaults"]["run"]["shell"], "bash")
        # Кэш зависимостей пишут push в ветку и ручной запуск; pull request и очередь читают.
        cache = steps_using(platforms, "Swatinem/rust-cache")[0]
        assert_pinned(self, RELEASE_WORKFLOW, cache, "v2")
        self.assertEqual(
            normalized(cache["with"]["save-if"]),
            "${{ github.ref_type == 'branch' && (github.event_name == 'push' || github.event_name == 'workflow_dispatch') }}",
        )
        assert_pinned(self, RELEASE_WORKFLOW, steps_using(platforms, "actions/setup-python")[0], "v7")
        # Консоль Windows — cp1252; сбой Windows виден, но упаковку ночи не блокирует.
        self.assertEqual(platforms["env"]["PYTHONUTF8"], "1")
        # Отладочная информация тестовой сборки — одна на джобу: шаг с другим
        # значением пересобрал бы общие артефакты.
        self.assertEqual(platforms["env"]["CARGO_PROFILE_DEV_DEBUG"], "line-tables-only")
        for step in platforms["steps"]:
            self.assertNotIn("CARGO_PROFILE_DEV_DEBUG", step.get("env") or {})
        self.assertEqual(normalized(platforms["continue-on-error"]), "${{ matrix.runner == 'windows-latest' }}")
        self.assertFalse((WORKFLOWS_DIR / "unica-large.yml").exists())

    def test_pages_take_results_from_the_queue_red_runs_and_the_nightly(self) -> None:
        """Отчёт `main` приходит с очереди; красный прогон — тоже результат; ночь и тег — тоже источники."""
        on = triggers(self.pages)
        build = job(self.pages, "build")

        self.assertEqual(on["workflow_run"]["workflows"], ["Build Unica Codex Plugin", "Unica Nightly"])
        self.assertEqual(on["workflow_run"]["branches"], ["main", "gh-readonly-queue/main/*", "release-v*", "v*"])
        self.assertIn("github.event.workflow_run.conclusion == 'failure'", condition(build))
        self.assertIn("github.event.workflow_run.event == 'merge_group'", condition(build))
        # Push в `main` тестов не гоняет и источником не бывает; push в релизную линию и тег — бывают.
        self.assertIn("(github.event.workflow_run.event == 'push' && github.event.workflow_run.head_branch != 'main')", normalized(condition(build)))
        self.assertIn("github.event.workflow_run.event == 'schedule'", condition(build))
        self.assertIn("github.event.workflow_run.head_repository.full_name == github.repository", condition(build))
        # Критерии конвейера сайт больше не считает: страница снята 08.09.2026.
        self.assertNotIn("--metrics", script(build))
        self.assertFalse((REPO_ROOT / "docs" / "pages" / "pipeline.html").exists())
        # Очередь даёт по прогону на каждый pull request партии, и деплои идут
        # друг за другом: неудачный деплой повторяется один раз после паузы.
        deploy = job(self.pages, "deploy")
        attempts = steps_using(deploy, "actions/deploy-pages")
        self.assertEqual(len(attempts), 2)
        self.assertTrue(attempts[0].get("continue-on-error"))
        self.assertEqual(normalized(attempts[1]["if"]), "steps.deployment.outcome == 'failure'")
        # Прямой триггер push снят: в очереди без отмены он заменял бы ожидающий
        # прогон с результатами, и они не доехали бы до сайта.
        self.assertNotIn("push", on)
        for found in jobs(self.pages).values():
            for expression in expressions(found):
                self.assertNotIn("github.event_name == 'push'", expression)
        self.assertIn("jobs?per_page=100", script(build))

    def test_pages_build_main_once_the_queue_merged_its_tree(self) -> None:
        """Сайт ждёт вливания проверенного очередью дерева и собирается из main, а не из коммита очереди.

        Без ожидания сайт брал прежний main и отставал на одно вливание. Из
        коммита очереди собирать нельзя: снятый с очереди pull request тоже даёт
        успешный прогон, а прогоны пачки кончаются не по порядку. Любой источник
        берёт main, какой он сейчас: прогон, отложенный группой, иначе выложил
        бы main старше опубликованного.
        """
        build = job(self.pages, "build")
        wait = step_named(build, "Дождаться вливания проверенного очередью дерева")
        self.assertEqual(
            condition(wait),
            "github.event.workflow_run.event == 'merge_group' && github.event.workflow_run.conclusion == 'success'",
        )
        self.assertIs(wait["continue-on-error"], True)
        self.assertIn("scripts/ci/await-queue-merge.py", wait["run"])
        self.assertEqual(wait["env"]["QUEUE_SHA"], "${{ github.event.workflow_run.head_sha }}")
        self.assertEqual(wait["env"]["QUEUE_BRANCH"], "${{ github.event.workflow_run.head_branch }}")

        # Скрипт ожидания исполняется из дерева события, страницы — из main;
        # коммит очереди не попадает в checkout никогда.
        first, current = steps_using(build, "actions/checkout")
        self.assertNotIn("ref", first["with"])
        self.assertEqual(current["with"]["ref"], "main")
        self.assertNotIn("if", current)
        for checkout in (first, current):
            self.assertIs(checkout["with"]["persist-credentials"], False)
        # Второй checkout чистит рабочий каталог: до него — только то, что его
        # переживает (Python в tool cache) или нужно для ожидания.
        order = steps(build)
        self.assertEqual(
            order[: order.index(current)],
            [first, steps_using(build, "actions/setup-python")[0], wait],
        )

        # Подвал называет дерево, из которого собраны страницы, и ведёт на него.
        rebuild = step_named(build, "Пересобрать страницы с итоговым статусом")
        self.assertIn('--sha "$(git rev-parse HEAD)"', rebuild["run"])
        self.assertNotIn("--run-url", rebuild["run"])

    def test_pages_runs_the_build_skips_stay_out_of_the_publishing_group(self) -> None:
        """Прогон, который build отсечёт, не вытесняет ожидающий прогон с результатами."""
        group = self.pages["concurrency"]["group"]
        build = condition(job(self.pages, "build"))
        expected = "${{ (" + build + ") && 'unica-pages' || format('unica-pages-idle-{0}', github.run_id) }}"
        self.assertEqual(normalized(group), normalized(expected))
        self.assertIs(self.pages["concurrency"]["cancel-in-progress"], False)

    def test_publication_queue_keeps_every_admitted_release(self) -> None:
        """Допущенные публикации ждут в одной очереди, пропущенные gate прогоны в неё не входят.

        Публикацию запускает каждая сборка, а gate пропускает только push
        тега и ручной запуск. Пустой прогон в общей группе снимал бы
        ожидающий выпуск, а с одним местом ожидания третий допущенный прогон
        снимал бы второй. Порядок внутри очереди GitHub не обещает: его держит
        страж порядка версий в stage и promote.
        """
        gate = job(self.publish, "gate")
        expected_group = (
            "${{ (" + condition(gate) + ") && 'publish-unica-marketplace' || "
            "format('publish-unica-marketplace-idle-{0}', github.run_id) }}"
        )
        self.assertEqual(normalized(self.publish["concurrency"]["group"]), normalized(expected_group))
        self.assertEqual(self.publish["concurrency"]["queue"], "max")
        self.assertIs(self.publish["concurrency"]["cancel-in-progress"], False)

    def test_guards_ship_findings_to_code_scanning_not_the_gate(self) -> None:
        """Находка линтера — не исход теста: SARIF в Code Scanning, гейт не краснеет."""
        guards = job(self.release, "guards")
        sarif = steps_using(guards, "github/codeql-action/upload-sarif")[0]

        self.assertIn("zizmor@1.30.0", [step["with"]["tool"] for step in steps_using(guards, "taiki-e/install-action")])
        self.assertIn("zizmor --config .github/zizmor.yml --format sarif --no-exit-codes .github/workflows > zizmor.sarif", script(guards))
        assert_pinned(self, RELEASE_WORKFLOW, sarif, "v4")
        self.assertEqual(sarif["with"]["category"], "zizmor")
        self.assertEqual(guards["permissions"].get("security-events"), "write")
        # Токен pull request из форка писать в Code Scanning не вправе.
        self.assertIn("github.event.pull_request.head.repo.full_name == github.repository", condition(sarif))

    def test_platform_build_uses_exact_cargo_cache_and_reports_outcome(self) -> None:
        build = job(self.release, "build-tools")
        step_by_id(build, "rust-toolchain")
        cache = step_by_id(build, "cargo-cache")

        self.assertIs(cache.get("continue-on-error"), True)
        assert_pinned(self, RELEASE_WORKFLOW, cache, "v6")
        self.assertEqual(cache["with"]["path"], ".build/tool-work/${{ matrix.target }}/cargo-target")
        self.assertEqual(
            normalized(cache["with"]["key"]),
            "cargo-${{ runner.os }}-${{ matrix.target }}-${{ steps.rust-toolchain.outputs.cachekey }}-${{ hashFiles('Cargo.lock') }}",
        )
        self.assertNotIn("restore-keys", cache["with"])
        first_build = next(index for index, step in enumerate(steps(build)) if "scripts/ci/build-unica-tools.py" in step.get("run", ""))
        self.assertLess(steps(build).index(cache), first_build)
        self.assertIn("--metrics-file", script(build))
        report = step_named(build, "Report Cargo cache and build metrics")
        self.assertEqual(condition(report), "always()")
        self.assertIn("steps.cargo-cache.outcome", " ".join(expressions(build)))
        self.assertIn("steps.cargo-cache.outputs.cache-hit", " ".join(expressions(build)))
        for outcome in ("exact-hit", "miss", "error"):
            with self.subTest(outcome=outcome):
                self.assertIn(outcome, report["run"])
        self.assertIn("cargoBuildSeconds", script(build))
        self.assertIn("archiveDownloadSeconds", script(build))
        self.assertIn("RLM archive download duration", script(build))
        self.assertIn("GITHUB_STEP_SUMMARY", script(build))

    def test_runtime_matrix_builds_verifies_and_exports_narrow_artifacts(self) -> None:
        build = job(self.release, "build-tools")
        names = [upload.get("name") for upload in uploads(build)]
        paths = [upload.get("path") for upload in uploads(build)]

        self.assertEqual({target for target, _ in targets(build)}, {"darwin-arm64", "linux-x64", "win-x64"})
        self.assertNotIn("package-runtime", jobs(self.release))
        self.assertFalse(any("unica-tools-" in value for value in strings(self.release)))
        for tool in ("scripts/ci/build-unica-tools.py", "scripts/ci/package-unica-runtime.py", "scripts/ci/verify-release-assets.py"):
            self.assertIn(tool, script(build))
        self.assertIn('--target "${{ matrix.target }}"', script(build))
        for name in ("unica-metadata-${{ matrix.target }}", "unica-bootstrap-${{ matrix.target }}", "unica-runtime-${{ matrix.target }}"):
            with self.subTest(name=name):
                self.assertIn(name, names)
        # Узость здесь — про цель, а не про артефакт: разрез поставки дал по
        # архиву на артефакт, и выгрузка обязана нести их все.
        for path in (
            ".build/runtime-assets/${{ matrix.target }}/*-runtime-${{ matrix.target }}.json",
            ".build/runtime-assets/${{ matrix.target }}/*-runtime-${{ matrix.target }}.tar.gz",
            ".build/bootstrap-artifacts/${{ matrix.target }}",
        ):
            with self.subTest(path=path):
                self.assertTrue(any(path in uploaded for uploaded in paths), path)
        self.assertIn("matrix.target == 'linux-x64'", " ".join(expressions(build)))
        self.assertIn("startsWith(github.ref, 'refs/tags/')", " ".join(expressions(build)))
        self.assertGreaterEqual(sum(1 for upload in uploads(build) if upload.get("retention-days") == 1), 3)

    def test_mcp_smoke_runs_against_extracted_deterministic_runtime(self) -> None:
        build = job(self.release, "build-tools")
        smoke = step_named(build, "Smoke packaged Unica MCP")

        self.assertLess(step_index(build, "Package deterministic runtime"), step_index(build, "Extract deterministic runtime for MCP smoke"))
        self.assertLess(step_index(build, "Extract deterministic runtime for MCP smoke"), step_index(build, "Smoke packaged Unica MCP"))
        self.assertIn('runtime_root=".build/runtime-smoke/${{ matrix.target }}"', script(build))
        self.assertIn(
            'tar -xzf ".build/runtime-assets/${{ matrix.target }}/unica-runtime-${{ matrix.target }}.tar.gz"',
            script(build),
        )
        self.assertIn('--plugin-root "$runtime_root"', script(build))
        self.assertIn('executable="$runtime_root/bin/${{ matrix.target }}/unica"', script(build))
        self.assertEqual(smoke.get("timeout-minutes"), 3)
        self.assertIn("--total-timeout-seconds 120", smoke["run"])

    def test_thin_payload_downloads_only_metadata_and_bootstrap(self) -> None:
        thin = job(self.release, "package-thin")
        patterns = [download.get("pattern") for download in downloads(thin)]
        marketplace = next(upload for upload in uploads(thin) if upload.get("name") == "unica-thin-marketplace")

        self.assertEqual(needs(thin), ["build-tools"])
        self.assertIn("unica-metadata-*", patterns)
        self.assertIn("unica-bootstrap-*", patterns)
        self.assertNotIn("unica-tools-*", patterns)
        self.assertNotIn("unica-runtime-*", patterns)
        self.assertIn("scripts/ci/package-unica-plugin.py", script(thin))
        self.assertIn("--runtime-metadata-root", script(thin))
        self.assertIn("--bootstrap-root", script(thin))
        self.assertIs(marketplace.get("include-hidden-files"), True)
        self.assertEqual(marketplace.get("retention-days"), 90)
        self.assertNotIn("unica-codex-marketplace-${{ matrix.target }}", list(strings(self.release)))

    def test_package_and_publication_file_gates_require_all_three_manifests(self) -> None:
        gates = (
            (job(self.release, "package-thin"), "dist/thin/marketplace"),
            (job(self.publish, "stage"), "payload"),
            (job(self.publish, "promote"), "payload"),
        )
        manifests = (
            "plugins/unica/.codex-plugin/plugin.json",
            "plugins/unica/.claude-plugin/plugin.json",
            "plugins/unica/.zcode-plugin/plugin.json",
        )
        for node, prefix in gates:
            gate = next(step for step in steps(node) if f"test -f {prefix}/{manifests[0]}" in step.get("run", ""))
            with self.subTest(job=prefix, step=gate["name"]), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                for relative in (*manifests, "plugins/unica/.mcp.json", "plugins/unica/runtime-manifest.json", ".agents/plugins/marketplace.json", ".claude-plugin/marketplace.json"):
                    target = root / prefix / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text("{}\n", encoding="utf-8")
                # Only the download is replaced; execute the workflow's file gate.
                shell = "gh() { :; }\n" + gate["run"]
                valid = subprocess.run(["bash", "-e", "-c", shell], cwd=root, capture_output=True, text=True, check=False)
                self.assertEqual(valid.returncode, 0, valid.stderr)
                for relative in manifests:
                    with self.subTest(missing=relative):
                        target = root / prefix / relative
                        target.unlink()
                        missing = subprocess.run(["bash", "-e", "-c", shell], cwd=root, capture_output=True, text=True, check=False)
                        target.write_text("{}\n", encoding="utf-8")
                        self.assertNotEqual(missing.returncode, 0, f"{gate['name']} accepted missing {relative}")

    def test_intermediate_non_marketplace_artifacts_expire_after_one_day(self) -> None:
        assessment = job(self.release, "release-assessment")

        self.assertEqual([(upload["name"], upload["retention-days"]) for upload in uploads(assessment)], [("unica-release-assessment", 1)])

    def test_packaged_bootstrap_is_smoked_on_every_supported_host(self) -> None:
        windows_cleanup = step_named(job(self.release, "build-tools"), "Check bootstrap smoke process cleanup on Windows")
        self.assertEqual(condition(windows_cleanup), "matrix.target == 'win-x64'")
        self.assertIn("tests.ci.test_smoke_unica_bootstrap", windows_cleanup["run"])
        probe = job(self.release, "probe-thin-bootstrap")
        smoke = job(self.release, "smoke-thin-plugin")
        expected_targets = {
            "linux-x64": "ubuntu-latest",
            "win-x64": "windows-2022",
            "darwin-arm64": "macos-14",
        }

        self.assertEqual(dict(targets(probe)), expected_targets)
        self.assertEqual(dict(targets(smoke)), expected_targets)
        step_named(probe, "Probe packaged bootstrap through the downloader")
        step_named(smoke, "Smoke packaged bootstrap against published runtime")
        self.assertIn("scripts/ci/smoke-unica-bootstrap.py", script(smoke))
        self.assertIn(" --plugin-root .build/thin/plugins/unica", script(smoke))
        self.assertIn(' --target "${{ matrix.target }}"', script(smoke))
        self.assertEqual(needs(probe), ["package-thin"])
        self.assertEqual(needs(smoke), ["package-thin", "publish-release-assets"])
        self.assertIn("--expect-download-failure", script(probe))

    def test_v080_source_release_has_no_executable_legacy_migration_jobs(self) -> None:
        for job_id in ("legacy-migration-preflight", "verify-installers", "installer"):
            with self.subTest(job_id=job_id):
                self.assertNotIn(job_id, jobs(self.release))
        for marker in ("test-unica-upgrade.ps1", "unica-installer", "install-unica.sh", "install-unica.ps1"):
            with self.subTest(marker=marker):
                self.assertFalse(any(marker in value for value in strings(self.release)), marker)

    def test_source_repo_has_no_manual_or_scheduled_full_migration_workflow(self) -> None:
        violations: dict[str, list[str]] = {}
        workflows = sorted((*WORKFLOWS_DIR.glob("*.yml"), *WORKFLOWS_DIR.glob("*.yaml")), key=lambda path: path.name)

        for workflow in workflows:
            text = workflow.read_text(encoding="utf-8")
            markers = [marker for marker in ("-Mode Full", "legacy-migration-full") if marker in text]
            if markers:
                violations[workflow.name] = markers

        self.assertFalse(LEGACY_WORKFLOW.exists())
        self.assertNotIn("unica-legacy-migration.yml", RELEASE_WORKFLOW.read_text(encoding="utf-8"))
        self.assertEqual({}, violations, f"source workflows own full migration policy: {violations}")

    def test_release_assets_are_published_without_pages_dependency_and_redownloaded(self) -> None:
        publish = job(self.release, "publish-release-assets")
        verify = job(self.release, "verify-published-assets")
        release = steps_using(publish, "softprops/action-gh-release")[0]

        self.assertNotIn("publish-assessment-pages", jobs(self.release))
        self.assertEqual(needs(publish), ["build-tools"])
        assert_pinned(self, RELEASE_WORKFLOW, release, "v3")
        self.assertIn("unica-runtime-*.tar.gz", release["with"]["files"])
        self.assertIn("unica-runtime-*.json", release["with"]["files"])
        # Паттерн скачивания `unica-runtime-*` обязан совпадать только с
        # артефактами выпуска: побочные артефакты (метаданные, верификация)
        # носят имена вне него, а не полагаются на фильтр `files:` (#701, п. 4).
        upload_names = {
            upload["name"]
            for job_id in jobs(self.release)
            for upload in uploads(job(self.release, job_id))
            if upload.get("name")
        }
        matched = {name for name in upload_names if fnmatch.fnmatch(name, "unica-runtime-*")}
        self.assertEqual(matched, {"unica-runtime-${{ matrix.target }}"})
        downloaded = steps_using(publish, "actions/download-artifact")
        self.assertEqual(
            [(step["with"].get("pattern"), step["with"].get("path")) for step in downloaded],
            [("unica-runtime-*", "dist/runtime"), ("unica-metadata-*", "dist/metadata")],
        )
        pair = step_named(publish, "Pair core archives with their metadata")["run"]
        self.assertIn('test -f "dist/metadata/unica-runtime-${target}.json"', pair)
        self.assertIn('cp "dist/metadata/unica-runtime-${target}.json" dist/runtime/', pair)
        self.assertIn("verify-release-assets.py --asset-dir dist/runtime", pair)
        self.assertFalse(any("install-unica" in value for value in strings(publish)))
        self.assertIn("gh release download", script(verify))
        self.assertIn("verify-release-assets.py", script(verify))

    def test_release_notes_are_generated_without_repository_docs(self) -> None:
        release = steps_using(job(self.release, "publish-release-assets"), "softprops/action-gh-release")[0]

        self.assertIs(release["with"].get("generate_release_notes"), True)
        self.assertNotIn("body_path", release["with"])
        self.assertFalse(any("docs/releases" in value for value in strings(self.release)))

    def test_assessment_is_independent_from_runtime_publication(self) -> None:
        assessment = job(self.release, "release-assessment")
        upload = steps_using(assessment, "actions/upload-artifact")[0]

        self.assertIn("always()", condition(assessment))
        self.assertIn("unica-runtime-linux-x64.tar.gz", "\n".join(strings(assessment)))
        self.assertNotIn("publish-release-assets", needs(assessment))
        self.assertFalse(any("publish-release-assets" in value for value in strings(assessment)))
        self.assertEqual(upload["with"]["name"], "unica-release-assessment")
        self.assertEqual(condition(upload), "always()")

    def test_pr_permissions_are_read_only_and_cross_repo_write_uses_secret(self) -> None:
        self.assertEqual(self.release["permissions"], {"contents": "read"})
        self.assertEqual(self.publish["permissions"], {"contents": "read"})
        tokens = {job_id: (found.get("env") or {}).get("GH_TOKEN") for job_id, found in jobs(self.publish).items()}
        self.assertIn("${{ secrets.UNICA_MARKETPLACE_TOKEN }}", tokens.values())
        for job_id, found in jobs(self.publish).items():
            with self.subTest(job_id=job_id):
                self.assertNotEqual((found.get("permissions") or {}).get("pull-requests"), "write")

    def test_cross_repository_push_configures_git_credentials(self) -> None:
        self.assertGreaterEqual(all_scripts(self.publish).count("gh auth setup-git"), 2)

    def test_publication_is_one_linear_pass_ordered_by_needs(self) -> None:
        """Publication order: stage → anchor → verify → promote, no pull requests, no warden.

        The order is the contract: the candidate anchor exists before the
        install checks run, and the catalog moves — with the release tag that
        it names — only behind their green result. A rerun of the whole
        workflow resumes a partial publication, so every stage states its
        idempotent escape.
        """
        on = triggers(self.publish)
        text = all_scripts(self.publish)
        gate = job(self.publish, "gate")

        self.assertIn("workflow_run", on)
        self.assertIn("source_run_id", on["workflow_dispatch"]["inputs"])
        # Сборка запускается и по push в main; публикацию открывает только тег.
        self.assertIn("startsWith(github.event.workflow_run.head_branch, 'v')", condition(gate))
        for job_id in ("stage", "anchor", "verify-fresh-install", "verify-upgrade", "promote"):
            with self.subTest(job_id=job_id):
                self.assertIn(job_id, jobs(self.publish))
        self.assertNotIn("tag", jobs(self.publish))
        self.assertEqual(needs(job(self.publish, "anchor")), ["stage"])
        self.assertEqual(needs(job(self.publish, "verify-fresh-install")), ["gate", "stage", "anchor"])
        self.assertEqual(needs(job(self.publish, "verify-upgrade")), ["gate", "stage", "anchor"])
        self.assertEqual(
            needs(job(self.publish, "promote")),
            ["gate", "stage", "anchor", "verify-fresh-install", "verify-upgrade"],
        )
        # The PR ceremony is gone with the warden: nothing opens pull requests
        # and no metadata travels in branch names.
        self.assertNotIn("pr create", text)
        self.assertNotIn("codex/stage-", text)
        self.assertNotIn("codex/promote-", text)
        self.assertNotIn("mode", on["workflow_dispatch"]["inputs"])
        stage_push = step_named(job(self.publish, "stage"), "Push the staged payload without changing any catalog")["run"]
        promote_move = step_named(
            job(self.publish, "promote"), "Publish the channel catalogs and the release tag in one push"
        )["run"]
        # Idempotent escapes: a completed stage and a completed promote are
        # detected, and an existing tag is proven identical, never moved. The
        # behaviour, reruns included, is exercised in test_publish_channels.
        self.assertIn('echo "staging_sha=$(git -C marketplace rev-parse HEAD)" >> "$GITHUB_OUTPUT"\n  exit 0', stage_push)
        self.assertIn('echo "${branch} already serves ${RELEASE_TAG}"', promote_move)
        self.assertIn('rev-parse --verify --quiet "refs/tags/${RELEASE_TAG}"', promote_move)
        self.assertNotIn("git tag -f", text)
        self.assertNotIn("--force", text)
        # The release tag is born only in promote, in the one atomic push that
        # moves the catalogs naming it; nothing else pushes a release tag.
        self.assertIn('git -C marketplace push --atomic origin "${refspecs[@]}"', promote_move)
        self.assertEqual(text.count('refs/tags/${RELEASE_TAG}")'), 1)
        self.assertNotIn('push origin "refs/tags/${RELEASE_TAG}"', text)
        # A stale straggler must fail forward-only instead of rolling a
        # catalog back — in both writers, over both host catalogs, in SemVer
        # order, and again on the retry after a refused push. That two
        # releases never interleave is
        # test_publication_queue_keeps_every_admitted_release.
        self.assertNotIn("sort -V", text)
        for writer in (stage_push, promote_move):
            with self.subTest(writer=writer.splitlines()[1]):
                self.assertIn('python3 "$rules" catalog-ref "marketplace/$codex" "marketplace/$claude"', writer)
                self.assertIn('python3 "$rules" forward', writer)
        self.assertIn('after_rebase="$(served_ref HEAD~1)"', stage_push)
        # The retry rebuilds the whole publication from the remote, which
        # repeats every forward check on the state it now holds.
        self.assertIn("if ! publish; then\n  prepare\n  publish\nfi", promote_move)
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
        self.assertIn("plugin remove $plugin --json", text)
        self.assertEqual(text.count("plugin add $plugin --json"), 3)
        # Each channel is installed under the name its consumers add.
        self.assertEqual(
            text.count("$plugin = if ($env:CHANNEL -eq 'next') { 'unica@unica-next' } else { 'unica@unica' }"),
            3,
        )
        self.assertIn("verify --plugin-root $pluginRoot", text)


class ArtifactSplitPublicationTests(unittest.TestCase):
    """Разрез поставки делит сборку и выкладку по-разному.

    Сборка несёт все артефакты: их метаданные нужны упаковщику, чтобы манифест
    объявил каждый. Выкладка несёт одно ядро: движки издал тулчейн, и вторая
    публикация тех же байтов стоила 439 МБ на выпуск.
    """

    def setUp(self) -> None:
        self.release = load(RELEASE_WORKFLOW)
        self.build = job(self.release, "build-tools")

    def test_the_release_publishes_the_core_and_only_it(self) -> None:
        # Выкладывается то, у чего есть читатель: пару ядра перекачивает и
        # перехеширует `verify-release-assets.py`. Описания поставок читает
        # только упаковщик, и берёт он их из артефакта сборки.
        release = steps_using(job(self.release, "publish-release-assets"), "softprops/action-gh-release")[0]

        self.assertIn("dist/runtime/unica-runtime-*.tar.gz", release["with"]["files"])
        self.assertIn("dist/runtime/unica-runtime-*.json", release["with"]["files"])
        self.assertFalse(any("dist/runtime/*-runtime-*" in value for value in strings(self.release)))

    def test_the_manifest_still_names_the_artifacts_the_release_does_not_carry(self) -> None:
        # Не выложить и не назвать — разные вещи. Движки объявлены адресом, и
        # каждый адрес выпуск проверяет.
        self.assertIn("verify-delivery-reachable.py", all_scripts(self.release))
        self.assertIn("prefetch --plugin-root", all_scripts(self.release))

    def test_packaging_uploads_every_artifact_of_the_target(self) -> None:
        paths = [upload.get("path", "") for upload in uploads(self.build)]
        for glob in (
            "runtime-assets/${{ matrix.target }}/*-runtime-${{ matrix.target }}.tar.gz",
            "runtime-assets/${{ matrix.target }}/*-runtime-${{ matrix.target }}.json",
        ):
            with self.subTest(glob=glob):
                self.assertTrue(any(glob in path for path in paths), glob)

    def test_bsp_runtime_assessment_receives_the_engine_its_search_requires(self) -> None:
        assessment = job(self.release, "release-assessment")

        self.assertIn("unica-assessment-engine-linux-x64", [upload.get("name") for upload in uploads(self.build)])
        self.assertIn("stage-unica-assessment-engine.py", script(self.build))
        self.assertIn("--artifact bsl-analyzer", script(self.build))
        self.assertIn("--artifact rlm-tools-bsl", script(self.build))
        self.assertIn("--out-archive .build/unica-assessment-engine-linux-x64.tar.gz", script(self.build))
        self.assertIn("unica-assessment-engine-linux-x64", [download.get("name") for download in downloads(assessment)])
        self.assertIn(
            "--engine-overlay .build/assessment-engine/unica-assessment-engine-linux-x64.tar.gz",
            script(assessment),
        )

    def test_the_direct_mcp_smoke_is_given_the_engines_it_asserts_on(self) -> None:
        extract = step_named(self.build, "Extract deterministic runtime for MCP smoke")

        self.assertIn("unica-runtime-${{ matrix.target }}.tar.gz", extract["run"])
        self.assertIn(".build/tool-bundles/${{ matrix.target }}/bin/", extract["run"])

    def test_every_supported_target_must_pass_before_publication(self) -> None:
        authoritative = targets(self.build)
        self.assertEqual(
            authoritative,
            [
                ("linux-x64", "ubuntu-latest"),
                ("win-x64", "windows-latest"),
                ("darwin-arm64", "macos-14"),
            ],
        )
        authoritative_targets = {target for target, _ in authoritative}
        for contour in ("probe-thin-bootstrap", "smoke-thin-plugin"):
            self.assertEqual({target for target, _ in targets(job(self.release, contour))}, authoritative_targets, contour)

        ordered_steps = (
            "Build target bundle and bootstrap",
            "Package deterministic runtime",
            "Verify local runtime asset pair",
            "Upload runtime metadata",
            "Upload bootstrap payload",
            "Upload required runtime archive",
        )
        positions = [step_index(self.build, name) for name in ordered_steps]
        self.assertEqual(positions, sorted(positions))
        self.assertIn("tools.json", script(self.build))
        self.assertIn('manifest["runtimeFiles"]', script(self.build))
        self.assertIn('--target "${{ matrix.target }}"', script(self.build))

        expected_needs = {
            "package-thin": ["build-tools"],
            "publish-release-assets": ["build-tools"],
            "probe-thin-bootstrap": ["package-thin"],
            "smoke-thin-plugin": ["package-thin", "publish-release-assets"],
            "verify-published-assets": ["publish-release-assets", "package-thin"],
        }
        for job_id, dependencies in expected_needs.items():
            found = job(self.release, job_id)
            self.assertEqual(needs(found), dependencies, job_id)
            for dependency in dependencies:
                self.assertIn(f"needs.{dependency}.result == 'success'", condition(found))

        local_verifier = "python scripts/ci/verify-release-assets.py"
        self.assertIn(local_verifier, script(self.build))
        self.assertIn('--asset-dir ".build/runtime-assets/${{ matrix.target }}"', script(self.build))

        published = job(self.release, "verify-published-assets")
        published_lifecycle = (
            'gh release download "$GITHUB_REF_NAME" --pattern \'unica-runtime-*\' --dir published',
            local_verifier + " --asset-dir published",
            "unica-thin-marketplace",
            "python scripts/ci/verify-delivery-reachable.py",
        )
        texts = [step_text(step) for step in steps(published)]
        published_positions = [next(index for index, text in enumerate(texts) if marker in text) for marker in published_lifecycle]
        self.assertEqual(published_positions, sorted(published_positions))
        self.assertNotIn("--target", "\n".join(texts), "published verification must cover every target")

        smoke = job(self.release, "smoke-thin-plugin")
        smoke_lifecycle = (
            "Smoke packaged bootstrap against published runtime",
            "Prefetch the whole delivery once, end to end",
        )
        smoke_positions = [step_index(smoke, name) for name in smoke_lifecycle]
        self.assertEqual(smoke_positions, sorted(smoke_positions))
        self.assertIn("matrix.target == 'linux-x64'", condition(step_named(smoke, "Prefetch the whole delivery once, end to end")))
        self.assertIn("prefetch --plugin-root .build/thin/plugins/unica", script(smoke))


class CandidateChannelTests(unittest.TestCase):
    """Кандидат выпуска раздаётся только каналу next, стабильный — обоим каналам.

    Замерить доставку можно только на настоящем релизе: адрес архива прибит к
    релизам репозитория. Кандидату `-rc.N` нужны тестировщики, и после выхода
    полной версии они должны получить её обычным обновлением. Какой канал
    получит выпуск, решает конвейер по тегу, а не память того, кто его запускал.
    Поведение каналов проверяет test_publish_channels, исполняя сами шаги.
    """

    def setUp(self) -> None:
        self.release = load(RELEASE_WORKFLOW)
        self.publish = load(PUBLISH_WORKFLOW)

    def test_a_prerelease_tag_marks_the_github_release_as_such(self) -> None:
        # Иначе предвыпуск станет «последним релизом» и его начнут находить
        # те, кто ищет свежее.
        release = steps_using(job(self.release, "publish-release-assets"), "softprops/action-gh-release")[0]

        self.assertEqual(normalized(release["with"]["prerelease"]), "${{ contains(github.ref_name, '-') }}")

    def test_publication_asks_first_which_channel_the_release_reaches(self) -> None:
        gate = job(self.publish, "gate")

        self.assertEqual(gate["outputs"], {"channel": "${{ steps.decide.outputs.channel }}"})
        self.assertIn(
            'python3 rules/scripts/ci/release-channel.py channel "$tag"',
            step_named(gate, "Decide which channel this release reaches")["run"],
        )

    def test_every_publishing_stage_waits_for_that_answer(self) -> None:
        # Достаточно загейтить первую стадию: остальные ждут её через `needs`.
        # Предвыпуск не `-rc.N` существует для замеров и не публикуется.
        stage = job(self.publish, "stage")

        self.assertEqual(needs(stage), ["gate"])
        self.assertEqual(
            condition(stage),
            "needs.gate.outputs.channel == 'stable' || needs.gate.outputs.channel == 'next'",
        )

    def test_channel_rules_come_from_the_workflow_commit_without_credentials(self) -> None:
        # Код тега не исполняется рядом с токеном маркетплейса: правила канала
        # берутся из коммита самого workflow, и checkout не оставляет токена.
        for job_id in ("gate", "stage", "anchor", "verify-fresh-install", "verify-upgrade", "promote"):
            with self.subTest(job_id=job_id):
                checkout = step_named(job(self.publish, job_id), "Check out the release channel rules")
                self.assertTrue(checkout["uses"].startswith("actions/checkout@"))
                self.assertEqual(
                    checkout["with"],
                    {
                        "ref": "${{ github.sha }}",
                        "path": "rules",
                        "persist-credentials": False,
                        "sparse-checkout": "scripts/ci/release-channel.py",
                        "sparse-checkout-cone-mode": False,
                    },
                )

if __name__ == "__main__":
    unittest.main()
