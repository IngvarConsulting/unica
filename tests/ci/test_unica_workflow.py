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
from pathlib import Path, PurePosixPath

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import ci_rust_lanes as lanes  # noqa: E402
import prepared_binary  # noqa: E402


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
RELEASE_WORKFLOW = WORKFLOWS_DIR / "release.yml"
CI_WORKFLOW = WORKFLOWS_DIR / "ci.yml"
CLIPPY_SCRIPT = REPO_ROOT / "scripts" / "ci" / "clippy.sh"
NIGHTLY_WORKFLOW = WORKFLOWS_DIR / "nightly.yml"
PAGES_WORKFLOW = WORKFLOWS_DIR / "release.yml"
PUBLISH_WORKFLOW = WORKFLOWS_DIR / "release.yml"
LEGACY_WORKFLOW = WORKFLOWS_DIR / "unica-legacy-migration.yml"
RELEASE_TAG_EXPRESSION = "${{ needs.alpha-tag.outputs.tag || (github.event_name == 'push' && github.ref_name) || '' }}"


def load_gate_module():
    import importlib.util
    spec = importlib.util.spec_from_file_location("evaluate_ci_gate", REPO_ROOT / "scripts/ci/evaluate-ci-gate.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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

        self.assertIn("cargo clippy --workspace --all-targets --all-features --message-format=json -- -D warnings", CLIPPY_SCRIPT.read_text(encoding="utf-8"))
        # Наборы гоняет шов; сами команды закреплены тестом `test_run_tests`.
        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem rust --phase run --prepared "$LANES" --results "$REPORT"', text)
        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem python --suite "$suite" --only-size "$lane" --docs-only "$DOCS_ONLY" --results', text)
        # Исполнение тестов остаётся у runner.
        gate = job(self.release, "unica-ci")
        for job_id, found in jobs(self.release).items():
            for step in steps(found):
                self.assertNotIn("cargo test", step.get("run", ""), job_id)
        self.assertNotIn("unittest discover", text)
        self.assertIn("python -m py_compile scripts/dev/*.py tests/dev/*.py", script(gate))
        self.assertIn("python scripts/ci/check-version-contract.py", script(gate))

    def test_label_changes_cannot_replace_the_stable_pull_request_gate(self) -> None:
        ci = load(CI_WORKFLOW)
        on = triggers(ci)
        ci_gate = job(ci, "unica-ci")
        gate = job(self.release, "unica-ci")

        # Изменение метки не должно создавать новый suite этого workflow:
        # ruleset смотрит на последний suite головы PR и перестаёт видеть
        # обязательный `Unica CI`, если тот suite состоит из skipped-джоб.
        self.assertEqual(on["pull_request"]["types"], ["opened", "synchronize", "reopened", "ready_for_review", "milestoned"])
        self.assertNotIn("paths", on["pull_request"])
        self.assertNotIn("label ignored", "\n".join(strings(self.release)))
        self.assertEqual(ci_gate["name"], "Unica CI")
        # Пропуск тестов без допуска ruleset считает пройденным; вливание блокирует
        # отдельная обязательная `PR metadata`, см. test_pr_metadata_workflow.
        self.assertEqual(normalized(ci_gate["if"]), "${{ needs.admission.outputs.admitted == 'true' }}")
        self.assertEqual(needs(ci_gate), ["admission"])
        self.assertEqual(gate["name"], "Unica CI")
        self.assertNotIn("if", gate)
        self.assertEqual(needs(gate), [])
        # Оценка — последний шаг и идёт всегда; она видит исходы всех шагов.
        evaluate = step_by_id(gate, "gate")
        self.assertIs(steps(gate)[-1], evaluate)
        self.assertEqual(condition(evaluate), "always()")
        self.assertEqual(evaluate["run"], "python scripts/ci/evaluate-ci-gate.py")
        self.assertEqual(evaluate["env"]["GATE_SCOPE"], "validation")
        self.assertEqual(normalized(evaluate["env"]["STEPS_JSON"]), "${{ toJSON(steps) }}")
        # Каждый шаг таблицы гейта существует под своим id: переименование
        # шага иначе стало бы «missing» только в красном прогоне.
        gate_module = load_gate_module()
        ids = {step.get("id") for step in steps(gate)}
        for entry, step_ids in gate_module.STEP_GROUPS.items():
            for step_id in step_ids:
                if step_id in gate_module.ABSENT_STEPS:
                    self.assertNotIn(step_id, ids)
                    continue
                with self.subTest(entry=entry, step=step_id):
                    self.assertIn(step_id, ids)
        # Шаги, которые идут после красного, обязаны идти и после него.
        for step_id in ("rust-run", "clippy", "python", "os-jobs", "verify", "removed-tests"):
            with self.subTest(step=step_id):
                self.assertIn("!cancelled()", condition(step_by_id(gate, step_id)))
        delivery = job(self.release, "delivery-ci")
        self.assertEqual(delivery["name"], "Delivery CI")
        self.assertIn("python scripts/ci/evaluate-ci-gate.py", script(delivery))
        self.assertEqual(steps(delivery)[-1]["env"]["GATE_SCOPE"], "delivery")
        for upstream in (
            "unica-ci",
            "rust-os",
            "build-tools",
            "package-thin",
            "probe-thin-bootstrap",
            "release-assessment",
            "publish-release-assets",
            "smoke-thin-plugin",
            "verify-published-assets",
            "p0-release-proof",
            "alpha-tag",
        ):
            with self.subTest(upstream=upstream):
                self.assertIn(upstream, needs(delivery))
        self.assertEqual(condition(delivery), "${{ always() }}")

    def test_build_workflow_does_not_subscribe_to_label_or_milestone_changes(self) -> None:
        # Проверяем все входы сборочного workflow: безопасный pull_request
        # не исключает параллельный pull_request_target с metadata types.
        self.assertEqual(metadata_build_triggers(self.release), {})

    def test_p0_dry_release_proof_is_read_only_and_aggregated(self) -> None:
        proof = job(self.release, "p0-release-proof")

        self.assertEqual(set(needs(proof)), {"build-tools", "package-thin", "release-assessment", "alpha-tag"})
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
        self.assertIn("p0-release-proof", needs(job(self.release, "delivery-ci")))

    def test_wire_probes_embed_the_matrix_target_in_their_evidence(self) -> None:
        build = job(self.release, "build-tools")
        for name in ("Probe native package wire profile", "Probe compatibility package wire profile"):
            with self.subTest(step=name):
                self.assertIn('--target "$TARGET"', step_text(step_named(build, name)))

    def test_classifier_exposes_typed_contours_and_event_routing(self) -> None:
        gate = job(self.release, "unica-ci")

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
                self.assertEqual(gate["outputs"][output], "${{ steps.classify.outputs." + output + " }}")
        scope = step_by_id(gate, "classify")
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
        # Джоба ОС идёт параллельно и маршрут считает тем же классификатором.
        other = step_by_id(job(self.release, "rust-os"), "classify")
        self.assertIn("python scripts/ci/classify-workflow-changes.py --github-event", other["run"])

    def test_every_classifier_call_gets_the_resolved_line(self) -> None:
        """Документационный пропуск пишет пометку с линией: пустая линия —
        отказ классификатора, и PR из одной прозы стал бы красным."""
        calls = 0
        for job_id, found in jobs(self.release).items():
            for step in steps(found):
                if "classify-workflow-changes.py" not in step.get("run", ""):
                    continue
                calls += 1
                with self.subTest(job=job_id):
                    self.assertIn('--line "$RUN_LINE"', step["run"])
                    self.assertEqual(step["env"]["RUN_LINE"], "${{ steps.line.outputs.line }}")
                    self.assertIn("line", {other.get("id") for other in steps(found)})
        self.assertEqual(calls, 3)

    def test_classifier_keeps_history_and_does_not_fetch_a_mutable_base(self) -> None:
        for job_id in ("unica-ci", "rust-os", "windows-clippy"):
            with self.subTest(job=job_id):
                found = job(self.release, job_id)
                checkout = steps_using(found, "actions/checkout")[0]
                scope = step_by_id(found, "classify")

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
        gate, other = job(self.release, "unica-ci"), job(self.release, "rust-os")
        for found, runner in ((gate, "ubuntu-latest"), (other, "$MATRIX_RUNNER")):
            build, run = step_by_id(found, "rust-build"), step_by_id(found, "rust-run")
            with self.subTest(runner=runner):
                for step, phase in ((build, "build"), (run, "run")):
                    self.assertIn("rust_required == 'true'", condition(step))
                    self.assertIn("--phase " + phase, step["run"])
                    self.assertIn('--prepared "$LANES" --results "$REPORT"', step["run"])
                    self.assertIn(f'--runner {runner}' if runner.startswith("ubuntu") else f'--runner "{runner}"', step["run"])
                    # Полосы выбирает ci_rust_lanes по профилю и ОС, не workflow.
                    self.assertNotIn("--rust-lane", step["run"])
                self.assertIn("!cancelled()", condition(run))
                order = steps(found)
                self.assertLess(order.index(build), order.index(run))
                # Планы выгружаются до тестов.
                plan_upload = next(step for step in steps_using(found, "actions/upload-artifact") if step["with"]["name"].startswith("plan-"))
                self.assertLess(order.index(build), order.index(plan_upload))
                self.assertLess(order.index(plan_upload), order.index(run))
                self.assertEqual(plan_upload["with"]["path"], ".build/ci-report/plan-*")
        verify = step_by_id(gate, "verify")
        self.assertIn("--phase verify", verify["run"])
        self.assertIn("CI_JOB_RESULTS", verify["run"])
        order = steps(gate)
        self.assertLess(order.index(step_by_id(gate, "os-jobs")), order.index(verify))
        self.assertIn("cargo fmt --all -- --check", step_by_id(gate, "guards-fmt")["run"])
        sarif = [step for step in steps_using(gate, "github/codeql-action/upload-sarif") if step["with"]["category"] != "zizmor"][0]
        self.assertEqual(sarif["with"]["category"], "clippy-ubuntu-latest")
        self.assertIn("always()", condition(sarif))
        self.assertIn("hashFiles('.build/clippy.sarif') != ''", condition(sarif))
        # Отчёт в Code Scanning — не проверка: сбой загрузки гейт не красит.
        self.assertIs(sarif["continue-on-error"], True)
        self.assertEqual(gate["permissions"], {"contents": "read", "actions": "read", "pull-requests": "read", "security-events": "write"})
        # Clippy исполняется в фоне, пока идут тесты; его шаг ждёт конца
        # и несёт его исход. Старт — до прогона, ожидание — до Python.
        start, clippy = step_by_id(gate, "clippy-start"), step_by_id(gate, "clippy")
        self.assertEqual(start["run"], "python scripts/ci/background_step.py start clippy -- scripts/ci/clippy.sh all")
        self.assertEqual(clippy["run"], "python scripts/ci/background_step.py wait clippy")
        self.assertNotIn("continue-on-error", clippy)
        order = steps(gate)
        self.assertLess(order.index(step_by_id(gate, "rust-build")), order.index(start))
        self.assertLess(order.index(start), order.index(step_by_id(gate, "rust-run")))
        self.assertLess(order.index(clippy), order.index(step_by_id(gate, "python")))
        self.assertIn("--all-targets --all-features --message-format=json -- -D warnings", CLIPPY_SCRIPT.read_text(encoding="utf-8"))
        # Джоба другой ОС линтует везде, где идёт.
        os_clippy = step_by_id(other, "clippy")
        self.assertEqual([os_clippy], [step for step in steps(other) if "cargo clippy" in step.get("run", "")])
        self.assertNotIn("if", other)
        # Матрица — один macOS; Windows только в `large`.
        self.assertEqual(
            normalized(other["strategy"]["matrix"]["runner"]),
            "${{ fromJSON(inputs.profile == 'large' && '[\"macos-15\", \"windows-latest\"]' || '[\"macos-15\"]') }}",
        )
        # Ни одна ОС не рекомендательная: красная джоба красит прогон (#2152).
        self.assertNotIn("continue-on-error", other)
        self.assertIn("rust_required == 'true'", condition(os_clippy))
        self.assertIn("--workspace --all-targets --all-features --locked -- -D warnings", os_clippy["run"])
        self.assertNotIn("continue-on-error", os_clippy)

    def test_os_job_lints_first_and_a_red_clippy_still_runs_the_tests(self) -> None:
        """Clippy джобы ОС — до сборки тестов; его отказ не отменяет сборку и прогон (#2152)."""
        other = job(self.release, "rust-os")
        order = steps(other)
        clippy, build, run = (step_by_id(other, step_id) for step_id in ("clippy", "rust-build", "rust-run"))
        cache = steps_using(other, "Swatinem/rust-cache")[0]
        plan_upload = next(step for step in steps_using(other, "actions/upload-artifact") if step["with"]["name"].startswith("plan-"))
        self.assertLess(order.index(cache), order.index(clippy))
        self.assertLess(order.index(clippy), order.index(build))
        self.assertIn("!cancelled()", condition(clippy))
        # Всё после Clippy идёт и после его отказа: оба сигнала — в одном прогоне.
        for step in (build, plan_upload, run):
            with self.subTest(step=step.get("id") or step["with"]["name"]):
                self.assertIn("!cancelled()", condition(step))

    def test_windows_clippy_runs_beside_the_main_profile_and_gates_it(self) -> None:
        """Ошибка Clippy, видимая только на Windows, не проходит кнопку `main` (#2152)."""
        lint = job(self.release, "windows-clippy")
        self.assertEqual(lint["name"], "Clippy (windows-latest)")
        self.assertEqual(lint["runs-on"], "windows-latest")
        self.assertEqual(lint["timeout-minutes"], 30)
        self.assertEqual(condition(lint), "${{ inputs.profile == 'main' }}")
        # Параллельно `Unica CI` и macOS: `needs` сделал бы прогон последовательным.
        self.assertEqual(needs(lint), [])
        self.assertNotIn("strategy", lint)
        self.assertNotIn("continue-on-error", lint)
        self.assertNotIn("permissions", lint)
        self.assertEqual(lint["defaults"]["run"]["shell"], "bash")
        # Ключ rust-cache включает переменные `CARGO*` и `RUST*`: они те же, что у джобы ОС.
        def cache_env(found: dict) -> dict:
            return {name: value for name, value in found.get("env", {}).items() if name.startswith(("CARGO", "RUST"))}
        self.assertEqual(cache_env(lint), cache_env(job(self.release, "rust-os")))
        self.assertTrue(cache_env(lint))
        classify = step_by_id(lint, "classify")
        self.assertIn("--runner-matrix '[\"windows-latest\"]'", classify["run"])
        toolchain = steps_using(lint, "dtolnay/rust-toolchain")
        self.assertEqual(len(toolchain), 1)
        self.assertEqual(toolchain[0]["with"], {"toolchain": "stable", "components": "clippy"})
        # Кэш только читается: ключ Windows пишет ночной `large`.
        cache = steps_using(lint, "Swatinem/rust-cache")
        self.assertEqual(len(cache), 1)
        assert_pinned(self, RELEASE_WORKFLOW, cache[0], "v2")
        self.assertEqual(cache[0]["with"]["shared-key"], "ci-rust-ordinary")
        self.assertIs(cache[0]["with"]["save-if"], False)
        clippy = step_by_id(lint, "clippy")
        self.assertEqual(clippy["run"], "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings")
        self.assertNotIn("continue-on-error", clippy)
        for step in (*toolchain, cache[0], clippy):
            self.assertIn("steps.classify.outputs.rust_required == 'true'", condition(step))
        # Гейт ждёт исход джобы целиком: без Rust она краснеет.
        required = step_named(lint, "Rust required")
        self.assertEqual(condition(required), "steps.classify.outputs.rust_required != 'true'")
        self.assertIn("exit 1", required["run"])
        self.assertLess(steps(lint).index(classify), steps(lint).index(required))
        self.assertLess(steps(lint).index(required), steps(lint).index(toolchain[0]))
        order = steps(lint)
        self.assertLess(order.index(classify), order.index(toolchain[0]))
        self.assertLess(order.index(toolchain[0]), order.index(cache[0]))
        self.assertLess(order.index(cache[0]), order.index(clippy))
        # `Unica CI` ждёт её по jobs API и без успеха краснеет.
        gate = job(self.release, "unica-ci")
        wait = step_by_id(gate, "windows-clippy")
        self.assertEqual(
            condition(wait),
            "${{ !cancelled() && steps.classify.outputs.rust_required == 'true' && inputs.profile == 'main' }}",
        )
        self.assertIn("python scripts/ci/await_os_jobs.py --runners '[]'", wait["run"])
        self.assertIn('--job "Clippy (windows-latest)"', wait["run"])
        self.assertIn("--require-success", wait["run"])
        self.assertEqual(wait["env"]["GH_TOKEN"], "${{ github.token }}")
        self.assertLess(steps(gate).index(wait), steps(gate).index(step_by_id(gate, "gate")))
        self.assertIn("windows-clippy", load_gate_module().STEP_GROUPS["lint-rust-windows"])

    def test_package_contour_and_pr_smoke_do_not_publish_release_assets(self) -> None:
        build = condition(job(self.release, "build-tools"))
        probe = condition(job(self.release, "probe-thin-bootstrap"))
        publish = condition(job(self.release, "publish-release-assets"))

        self.assertIn("needs.unica-ci.outputs.release_required == 'true'", build)
        self.assertIn("needs.unica-ci.outputs.ci_changed == 'true'", build)
        self.assertIn("needs.unica-ci.result == 'success'", build)
        self.assertIn("needs.rust-os.result == 'success'", build)
        # Неудача alpha не собирает непомеченное дерево как будто всё прошло.
        self.assertIn("(needs.alpha-tag.result == 'success' || needs.alpha-tag.result == 'skipped')", build)
        # Проба ждёт отказа скачивания: у выпуска и alpha ассеты уже есть.
        self.assertIn("github.event_name != 'push'", probe)
        self.assertIn("needs.alpha-tag.outputs.tag == ''", probe)
        # Публикует только тег или выпущенный alpha, кнопка — нет.
        self.assertIn("(github.event_name == 'push' || needs.alpha-tag.outputs.tag != '')", publish)

    def test_release_workflow_runs_by_tag_button_and_the_nightly_call(self) -> None:
        """Ворота pull request и очереди живут в `ci.yml`, здесь выпуск, кнопка и вызов ночи."""
        on = triggers(self.release)

        self.assertEqual(set(on), {"workflow_dispatch", "workflow_call", "push"})
        self.assertEqual(on["push"], {"tags": ["v*"]})
        self.assertEqual(on["workflow_dispatch"]["inputs"]["profile"]["options"], ["main", "large"])
        self.assertEqual(set(on["workflow_call"]["inputs"]), {"profile", "alpha", "force_alpha"})
        # Связи между workflow через завершение прогона больше нет нигде.
        for workflow in sorted(WORKFLOWS_DIR.glob("*.yml")):
            with self.subTest(workflow=workflow.name):
                self.assertNotIn("workflow_run", triggers(load(workflow)))

    def test_lint_and_test_failures_cannot_cancel_other_validation_jobs(self) -> None:
        gate = job(self.release, "unica-ci")
        # Красный шаг не отменяет следующие: тесты, Clippy, Python и сверка
        # идут после любого отказа, кроме отмены прогона.
        # Каждый шаг после первого идёт и после красного предыдущего: отказ
        # guards или selector не лишает прогон сборки, тестов и отчёта.
        for index, step in enumerate(steps(gate)):
            if index <= [found.get("id") for found in steps(gate)].index("guards-compile"):
                continue
            with self.subTest(step=step.get("id") or step.get("name") or step.get("uses")):
                gate_condition = condition(step)
                # Единственное исключение — база следующих прогонов: её
                # публикует только полностью зелёный прогон.
                if (step.get("with") or {}).get("name") == "ci-rust-inventory-${{ env.GATE_PROFILE }}":
                    self.assertIn("success()", gate_condition)
                    continue
                self.assertTrue("!cancelled()" in gate_condition or "always()" in gate_condition, gate_condition)
        # Внутри шага один красный набор или полоса не останавливает остальные.
        python = step_by_id(gate, "python")
        self.assertIn('wait "${pids[$index]}" || failed+=("${slugs[$index]}")', python["run"])
        self.assertNotIn("set -e", python["run"].replace("set -uo pipefail", ""))
        self.assertIs(job(self.release, "rust-os")["strategy"]["fail-fast"], False)
        build = job(self.release, "build-tools")
        self.assertEqual(needs(build), ["unica-ci", "rust-os", "alpha-tag"])
        self.assertIn("needs.unica-ci.result == 'success'", condition(build))
        # Результаты выгружаются и при красном прогоне.
        for found in (gate, job(self.release, "rust-os")):
            results = next(step for step in steps_using(found, "actions/upload-artifact") if step["with"]["name"].startswith("results-"))
            self.assertEqual(condition(results), "always()")

    def test_rust_cache_is_shared_without_a_lint_only_writer(self) -> None:
        for job_id in ("unica-ci", "rust-os"):
            found = job(self.release, job_id)
            cache = steps_using(found, "Swatinem/rust-cache")
            self.assertEqual(len(cache), 1)
            assert_pinned(self, RELEASE_WORKFLOW, cache[0], "v2")
            # Один ключ зависимостей на ОС: архивы, продукт и Clippy.
            self.assertEqual(cache[0]["with"]["shared-key"], "ci-rust-ordinary")
            self.assertEqual(
                normalized(cache[0]["with"]["save-if"]),
                "${{ github.ref_type == 'branch' }}",
            )
            self.assertIs(cache[0]["with"]["cache-on-failure"], True)
            self.assertNotIn("lookup-only", cache[0]["with"])
            self.assertNotIn("cache-workspace-crates", cache[0]["with"])
            # Кешируется тот каталог, куда собирают архивы и продукт: `<root>/target`.
            self.assertNotIn("workspaces", cache[0]["with"])
            self.assertEqual(found["env"]["CARGO_PROFILE_DEV_DEBUG"], "line-tables-only")
            for step in steps(found):
                self.assertNotIn("CARGO_PROFILE_DEV_DEBUG", step.get("env") or {})
            # nextest ставится до восстановления: rust-cache исключает уже
            # лежащие в ~/.cargo/bin файлы из записи и не подменяет их.
            order = steps(found)
            nextest = next(step for step in steps_using(found, "taiki-e/install-action") if "cargo-nextest@" in step["with"]["tool"])
            self.assertEqual(nextest["with"]["tool"], "cargo-nextest@0.9.148")
            self.assertLess(order.index(nextest), order.index(cache[0]))
        for lane in ("ordinary", "receipt", "vendor"):
            archive = lanes.archive_command(lane, Path("tests.tar.zst"))
            self.assertEqual(Path(archive[archive.index("--target-dir") + 1]), lanes.ROOT / "target")
        # Продукт для Python собирается в тот же target и копируется вне него.
        product = step_by_id(job(self.release, "unica-ci"), "product")
        self.assertIn("python scripts/ci/prepared_binary.py build --destination .build/ci-product --target-dir target", product["run"])
        self.assertIn("UNICA_PREPARED_BINARY_MANIFEST=$PWD/.build/ci-product/manifest.json", product["run"])
        self.assertNotIn("target", PurePosixPath(".build/ci-product").parts)
        gate = job(self.release, "unica-ci")
        script_text = CLIPPY_SCRIPT.read_text(encoding="utf-8")
        per_package = script_text[script_text.index("  packages)"):]
        # Признаки, объединённые workspace, прячут мёртвый код отдельного пакета.
        self.assertIn("cargo metadata --no-deps", per_package)
        # Отказ подстановки в заголовке `for` даёт ноль итераций и зелёный шаг.
        self.assertNotRegex(per_package, r"for \w+ in \$\(")
        self.assertIn('"${#packages[@]}" -eq 0', per_package)
        self.assertIn("--all-targets --locked -- -D warnings", per_package)
        self.assertNotIn("--all-features", per_package)
        self.assertNotIn("--workspace", per_package)
        self.assertIn("set -euo pipefail", script_text)
        self.assertEqual(
            [step["with"]["tool"] for step in steps_using(gate, "taiki-e/install-action")],
            ["zizmor@1.30.0", "cargo-nextest@0.9.148", "clippy-sarif@0.8.0,sarif-fmt@0.8.0"],
        )

    def test_release_assessment_uses_affected_mechanism_contour(self) -> None:
        assessment = job(self.release, "release-assessment")

        self.assertEqual(needs(assessment), ["unica-ci", "build-tools", "alpha-tag"])
        self.assertIn("needs.unica-ci.outputs.assessment_required == 'true'", condition(assessment))
        self.assertIn("needs.build-tools.result == 'success'", condition(assessment))

    def test_release_assessment_uses_the_candidate_release_identity(self) -> None:
        assessment = job(self.release, "release-assessment")
        resolve = step_named(assessment, "Resolve the release tag for non-tag builds")

        self.assertEqual(
            normalized(assessment["env"]["RELEASE_TAG"]),
            RELEASE_TAG_EXPRESSION,
        )
        self.assertEqual(condition(resolve), "${{ env.RELEASE_TAG == '' }}")
        self.assertIn('echo "RELEASE_TAG=v${version}" >> "$GITHUB_ENV"', resolve["run"])
        self.assertIn('--release-tag "$RELEASE_TAG"', script(assessment))
        self.assertNotIn("RELEASE_REF", assessment["env"])

    def test_only_tag_pushes_and_a_released_alpha_enable_release_behavior(self) -> None:
        for job_id in ("build-tools", "package-thin", "release-assessment", "publish-release-assets", "verify-published-assets"):
            with self.subTest(job_id=job_id):
                self.assertEqual(normalized(job(self.release, job_id)["env"]["RELEASE_TAG"]), RELEASE_TAG_EXPRESSION)
        self.assertIn("(github.event_name == 'push' || needs.alpha-tag.outputs.tag != '')", condition(job(self.release, "publish-release-assets")))
        for job_id in ("smoke-thin-plugin", "verify-published-assets"):
            with self.subTest(job_id=job_id):
                self.assertIn("needs.publish-release-assets.result == 'success'", condition(job(self.release, job_id)))

    def test_a_green_night_builds_and_publishes_the_alpha_in_the_same_run(self) -> None:
        alpha = job(self.release, "alpha-tag")
        self.assertEqual(needs(alpha), ["unica-ci", "rust-os"])
        gate = condition(alpha)
        for clause in ("inputs.alpha", "needs.unica-ci.result == 'success'", "needs.rust-os.result == 'success'"):
            self.assertIn(clause, gate)
        # Тег ставит и пушит только эта джоба, и только ей нужна запись в тег.
        self.assertEqual(alpha["permissions"], {"contents": "write"})
        self.assertIn("python scripts/ci/release-alpha.py publish", script(alpha))
        self.assertEqual(set(alpha["outputs"]), {"tag", "sha"})
        # Вся сборка идёт на служебном коммите: номер в коде равен тегу.
        sha = "${{ needs.alpha-tag.outputs.sha || github.sha }}"
        for job_id in ("build-tools", "package-thin", "p0-release-proof", "probe-thin-bootstrap", "release-assessment",
                       "publish-release-assets", "smoke-thin-plugin", "verify-published-assets"):
            with self.subTest(job_id=job_id):
                found = job(self.release, job_id)
                self.assertIn("alpha-tag", needs(found))
                self.assertEqual(steps_using(found, "actions/checkout")[0]["with"]["ref"], sha)
        self.assertIn(f'--source-commit "{sha}"', script(job(self.release, "package-thin")))
        # Второго прогона нет: от alpha не зависит ни один вызов workflow.
        self.assertNotIn("gh workflow run", all_scripts(self.release))
        self.assertNotIn("gh workflow run", (REPO_ROOT / "scripts/ci/release-alpha.py").read_text(encoding="utf-8"))
        # Маркетплейс alpha не получает: он ждёт push тега и канала.
        self.assertIn("github.event_name == 'push'", condition(job(self.release, "gate")))

    def test_an_alpha_publishes_the_full_set_and_cleans_up_after_itself(self) -> None:
        publish = job(self.release, "publish-release-assets")
        release = steps_using(publish, "softprops/action-gh-release")[0]
        self.assertIn("dist/full/unica-full-*.tar.gz", release["with"]["files"])
        self.assertIn("main@", "\n".join(strings(publish)))
        order = steps(publish)
        cleanup = next(step for step in order if "release-alpha.py prune" in str(step.get("run", "")))
        self.assertIs(cleanup["continue-on-error"], True)
        self.assertGreater(order.index(cleanup), order.index(release))
        package = step_named(job(self.release, "build-tools"), "Package the full alpha set")
        self.assertEqual(condition(package), "${{ contains(env.RELEASE_TAG, '-alpha.') }}")
        self.assertIn('--local-debug-target "$TARGET"', package["run"])

    def test_a_prerelease_uploads_into_a_draft_and_publishes_after_it(self) -> None:
        # Неизменяемый релиз после публикации файлов не принимает.
        publish = job(self.release, "publish-release-assets")
        release = steps_using(publish, "softprops/action-gh-release")[0]
        self.assertEqual(normalized(release["with"]["draft"]), "${{ contains(env.RELEASE_TAG, '-') }}")
        self.assertEqual(release["with"]["tag_name"], "${{ env.RELEASE_TAG }}")
        finish = step_named(publish, "Publish the prerelease draft")
        self.assertIn("--draft=false", finish["run"])
        self.assertGreater(steps(publish).index(finish), steps(publish).index(release))

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
            # Два часа воротам, пять — ночному `large`: Linux исполняет весь
            # набор подряд и ждёт Windows с полным набором.
            "unica-ci": "${{ inputs.profile == 'large' && 300 || 120 }}",
            "rust-os": "${{ inputs.profile == 'large' && 180 || 60 }}",
            "alpha-tag": 15,
            "windows-clippy": 30,
            "build-tools": 90,
            "package-thin": 30,
            "probe-thin-bootstrap": 30,
            "release-assessment": 60,
            "publish-release-assets": 15,
            "smoke-thin-plugin": 30,
            "verify-published-assets": 15,
            "delivery-ci": 5,
        }
        for job_id, minutes in expected_release_timeouts.items():
            with self.subTest(job_id=job_id):
                value = job(self.release, job_id).get("timeout-minutes")
                self.assertEqual(normalized(value) if isinstance(value, str) else value, minutes)
        # `Unica CI` ждёт джобу ОС не дольше её собственного срока: дальше —
        # `timed_out`, не вечность.
        wait = step_by_id(job(self.release, "unica-ci"), "os-jobs")
        self.assertIn('--timeout "$WAIT_SECONDS"', wait["run"])
        self.assertEqual(normalized(wait["env"]["WAIT_SECONDS"]),
                         "${{ inputs.profile == 'large' && 11400 || 3600 }}")

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
        """Наборы Python идут по матрице шва, в той же джобе после Rust."""
        gate = job(self.release, "unica-ci")
        python, plan = step_by_id(gate, "python"), step_by_id(gate, "python-plan")

        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem python --suite "$suite" --only-size "$lane" --docs-only "$DOCS_ONLY" --results', python["run"])
        # Матрицу наборов считает шов по воротам; шаг идёт по её строкам.
        for step in (python, plan):
            self.assertEqual(step["env"]["PYTHON_MATRIX"], "${{ steps.python-matrix.outputs.python_matrix }}")
            self.assertEqual(step["env"]["DOCS_ONLY"], "${{ steps.classify.outputs.docs_only }}")
            self.assertIn("jq -c '.[]' <<<\"$PYTHON_MATRIX\"", step["run"])
        self.assertEqual(gate["outputs"]["python_matrix"], "${{ steps.python-matrix.outputs.python_matrix }}")
        self.assertIn('run-tests.py --profile "$GATE_PROFILE" --python-matrix', step_by_id(gate, "python-matrix")["run"])
        self.assertEqual(step_by_id(gate, "python-matrix")["env"]["DOCS_ONLY"], "${{ steps.classify.outputs.docs_only }}")
        ids = [step.get("id") for step in steps(gate)]
        self.assertLess(ids.index("classify"), ids.index("python-matrix"))
        # План выгружается до тестов, как у Rust; каталоги по наборам.
        self.assertIn("--plan-only", plan["run"])
        self.assertIn('--results "$REPORT/plan-python-$slug"', plan["run"])
        self.assertIn('results="$REPORT/results-python-$slug"', python["run"])
        self.assertLess(ids.index("python-plan"), ids.index("python"))
        # Python не идёт параллельно с Rust: MCP-тесты под нагрузкой падают
        # по таймауту. Он идёт после тестов и Clippy и с проверенным продуктом.
        for step_id in ("product", "rust-run", "clippy"):
            self.assertLess(ids.index(step_id), ids.index("python"))
        # Наборы Python идут параллельно друг другу, и каждый ожидается: ни один
        # не остаётся фоновым после шага.
        self.assertIn('for index in "${!pids[@]}"; do', python["run"])
        self.assertEqual(python["run"].count(" &\n"), 1)

    def test_agent_evaluation_and_credential_are_absent_from_every_workflow(self) -> None:
        for workflow in sorted((*WORKFLOWS_DIR.glob("*.yml"), *WORKFLOWS_DIR.glob("*.yaml"))):
            values = list(strings(load(workflow)))
            with self.subTest(workflow=workflow.name):
                self.assertFalse(any("tests/agent_evaluation" in value for value in values))
                self.assertFalse(any("CODEX_API_KEY" in value for value in values))

    def test_gate_profile_follows_the_event_not_the_job(self) -> None:
        """Кнопка и ночь передают свой профиль, тег — `release`."""
        self.assertEqual(normalized(self.release["env"]["GATE_PROFILE"]), "${{ inputs.profile || 'release' }}")
        # Очередь слияния: ворота отвечают на merge_group, иначе очередь ждёт вечно.
        ci = load(CI_WORKFLOW)
        self.assertEqual(triggers(ci)["merge_group"], {"types": ["checks_requested"]})
        self.assertEqual(
            normalized(ci["env"]["GATE_PROFILE"]),
            "${{ github.event_name == 'merge_group' && 'queue' || 'pr' }}",
        )
        self.assertNotIn("run-tests.py --profile all", all_scripts(self.release))

    def test_line_rides_in_the_signature_and_tags_resolve_to_a_release_line(self) -> None:
        for job_id in ("unica-ci", "rust-os"):
            found = job(self.release, job_id)
            with self.subTest(job=job_id):
                self.assertIn('python scripts/ci/resolve-line.py --ref-type "$REF_TYPE" --ref-name "$REF_NAME" --sha "$GITHUB_SHA"', step_by_id(found, "line")["run"])
                for step_id in ("rust-build", "rust-run"):
                    step = step_by_id(found, step_id)
                    self.assertEqual(step["env"]["RUN_LINE"], "${{ steps.line.outputs.line }}")
                    self.assertEqual(1, step["run"].count('--line "$RUN_LINE"'))
        gate = job(self.release, "unica-ci")
        self.assertEqual(gate["outputs"]["line"], "${{ steps.line.outputs.line }}")
        for step_id in ("python", "python-plan"):
            step = step_by_id(gate, step_id)
            self.assertEqual(step["env"]["RUN_LINE"], "${{ steps.line.outputs.line }}")
            self.assertEqual(1, step["run"].count('--line "$RUN_LINE"'))
        for found in jobs(self.release).values():
            for upload in uploads(found):
                self.assertNotEqual(upload.get("path"), ".build/results/plan.json")

    def test_the_nightly_calls_the_release_workflow_with_the_large_profile(self) -> None:
        """Ночь проверяет то, что выпускается: один прогон, без запуска другого workflow."""
        decide, full = job(self.nightly, "decide"), job(self.nightly, "full")

        self.assertIn("schedule", triggers(self.nightly))
        self.assertIn("workflow_dispatch", triggers(self.nightly))
        # Круглые минуты GitHub исполняет с наибольшим опозданием.
        minute = triggers(self.nightly)["schedule"][0]["cron"].split()[0]
        self.assertNotIn(minute, ("0", "30"))
        self.assertIn("python scripts/ci/nightly-decide.py", script(decide))
        self.assertEqual(self.nightly["permissions"], {"contents": "read"})
        self.assertEqual(self.nightly["concurrency"]["group"], "unica-nightly")
        self.assertEqual(full["uses"], "$/.github/workflows/release.yml")
        self.assertEqual(needs(full), ["decide"])
        self.assertEqual(condition(full), "${{ needs.decide.outputs.run == 'true' }}")
        self.assertEqual(full["with"]["profile"], "large")
        # Alpha — только срез main: кнопка на другой ветке его не выпускает.
        self.assertEqual(full["with"]["alpha"], "${{ github.ref == 'refs/heads/main' }}")
        # Права вызова — верхняя граница: их хватает всем заданиям `release.yml`, но
        # токен маркетплейса вызову не передаётся, а `id-token` нужен только Pages.
        self.assertNotIn("secrets", full)
        requested = {}
        for found in jobs(self.release).values():
            requested.update(found.get("permissions") or {})
        for scope, level in requested.items():
            with self.subTest(scope=scope):
                self.assertIn(scope, full["permissions"])
                if level == "write":
                    self.assertEqual(full["permissions"][scope], "write")
        # Запуска другого workflow нет: итог ночи — итог этого прогона.
        self.assertNotIn("gh workflow run", all_scripts(self.nightly))
        self.assertNotIn("continue-on-error", full)

    def test_every_nightly_guard_stays_in_the_called_workflow(self) -> None:
        """Windows, macOS, тяжёлые тесты, отчёты, сборка, инвентарь и снятые тесты."""
        gate = job(self.release, "unica-ci")
        self.assertIn("windows-latest", job(self.release, "rust-os")["strategy"]["matrix"]["runner"])
        self.assertIn("macos-15", job(self.release, "rust-os")["strategy"]["matrix"]["runner"])
        self.assertEqual(load_gate_module().PROFILE_RUNNERS["large"], ["ubuntu-latest", "macos-15", "windows-latest"])
        self.assertEqual(
            {step["with"]["category"] for step in steps_using(gate, "github/codeql-action/upload-sarif")},
            {"zizmor", "clippy-ubuntu-latest"},
        )
        self.assertIn("python scripts/ci/removed_tests.py --baseline .build/ci-baseline/baseline.json", step_by_id(gate, "removed-tests")["run"])
        self.assertIn("--phase verify", step_by_id(gate, "verify")["run"])
        for job_id in ("build-tools", "package-thin", "delivery-ci"):
            self.assertIn(job_id, jobs(self.release))

    def test_pages_are_jobs_of_the_run_that_reports_them(self) -> None:
        """Сайт обновляется заданием прогона: и зелёного, и красного, но не чужой ветки."""
        build, deploy = job(self.release, "pages-build"), job(self.release, "pages-deploy")

        # После итога сборки и маркетплейса, даже красных.
        self.assertEqual(needs(build), ["delivery-ci", "promote"])
        gate = condition(build)
        self.assertIn("!cancelled()", gate)
        self.assertNotIn("result == 'success'", gate)
        self.assertIn("github.repository == 'IngvarConsulting/unica'", gate)
        self.assertIn("github.ref == 'refs/heads/main'", gate)
        self.assertIn("startsWith(github.ref_name, 'release-v')", gate)
        self.assertIn("github.ref_type == 'tag'", gate)
        # Результаты — свои, артефакты чужого прогона больше не читаются.
        download = steps_using(build, "actions/download-artifact")[0]
        self.assertNotIn("run-id", download["with"])
        self.assertIn('actions/runs/$GITHUB_RUN_ID/jobs', script(build))
        self.assertNotIn("workflow_run", "\n".join(strings(build)))
        # Страницы всегда из main: сайт двигается только вперёд.
        checkout = step_named(build, "Взять main")
        self.assertEqual(checkout["with"]["ref"], "main")
        self.assertEqual(steps_using(build, "actions/checkout"), [checkout])
        self.assertEqual(build["permissions"], {"contents": "read", "actions": "read"})
        # Право писать на Pages есть только у публикации, и тег alpha её не проходит.
        self.assertEqual(needs(deploy), ["pages-build"])
        self.assertEqual(deploy["permissions"], {"pages": "write", "id-token": "write"})
        self.assertEqual(deploy["environment"]["name"], "github-pages")
        self.assertIn("!contains(github.ref_name, '-alpha.')", condition(deploy))
        self.assertIn("needs.pages-build.result == 'success'", condition(deploy))
        for found in (build, deploy):
            self.assertEqual(found["concurrency"], {"group": "unica-pages", "cancel-in-progress": False})

    def test_publication_queue_keeps_every_admitted_release(self) -> None:
        """Прогоны тегов ждут в одной очереди, проверки веток в неё не входят.

        С одним местом ожидания третий выпуск снимал бы второй. Порядок внутри
        очереди GitHub не обещает: его держит страж порядка версий в stage и promote.
        """
        concurrency = self.release["concurrency"]
        self.assertEqual(
            normalized(concurrency["group"]),
            "${{ github.ref_type == 'tag' && 'unica-release-tags' || format('unica-check-{0}-{1}', github.ref, inputs.profile) }}",
        )
        self.assertEqual(concurrency["queue"], "max")
        self.assertIs(concurrency["cancel-in-progress"], False)

    def test_guards_ship_findings_to_code_scanning_not_the_gate(self) -> None:
        """Находка линтера — не исход теста: SARIF в Code Scanning, гейт не краснеет."""
        gate = job(self.release, "unica-ci")
        sarif = next(step for step in steps_using(gate, "github/codeql-action/upload-sarif") if step["with"]["category"] == "zizmor")

        self.assertIn("zizmor@1.30.0", [step["with"]["tool"] for step in steps_using(gate, "taiki-e/install-action")])
        self.assertIn("zizmor --config .github/zizmor.yml --format sarif --no-exit-codes .github/workflows > .build/zizmor.sarif", step_by_id(gate, "guards-zizmor")["run"])
        assert_pinned(self, RELEASE_WORKFLOW, sarif, "v4")
        self.assertEqual(sarif["with"]["sarif_file"], ".build/zizmor.sarif")
        self.assertIs(sarif["continue-on-error"], True)
        self.assertEqual(gate["permissions"].get("security-events"), "write")
        # Отчёты лежат вне дерева исходников: неотслеживаемый файл сменил бы
        # отпечаток, к которому привязан продукт для Python.
        for step in steps(gate):
            for written in re.findall(r"(?:>|tee) (\S+\.sarif)", step.get("run", "")):
                self.assertTrue(written.startswith(".build/"), written)

    def test_platform_build_uses_exact_cargo_cache_and_reports_outcome(self) -> None:
        build = job(self.release, "build-tools")
        step_by_id(build, "rust-toolchain")
        cache = step_by_id(build, "cargo-cache")

        self.assertIs(cache.get("continue-on-error"), True)
        assert_pinned(self, RELEASE_WORKFLOW, cache, "v6")
        self.assertEqual(cache["with"]["path"], ".build/tool-work/${{ matrix.target }}/cargo-target")
        self.assertEqual(
            normalized(cache["with"]["key"]),
            "cargo-${{ runner.os }}-${{ matrix.target }}-${{ steps.rust-toolchain.outputs.cachekey }}-${{ steps.lock.outputs.hash }}",
        )
        # Номер alpha в Cargo.lock не меняет ключ: иначе каждая ночь писала бы новую запись.
        self.assertIn("grep -v -- '-alpha", step_by_id(build, "lock")["run"])
        self.assertLess(steps(build).index(step_by_id(build, "lock")), steps(build).index(cache))
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
        self.assertIn("env.RELEASE_TAG != ''", " ".join(expressions(build)))
        self.assertGreaterEqual(sum(1 for upload in uploads(build) if upload.get("retention-days") == 1), 3)

    def test_manual_macos_acceptance_exports_core_bytes_without_publishing_a_release(self) -> None:
        build = job(self.release, "build-tools")
        upload = step_named(build, "Upload required runtime archive")

        def allowed(node, target, event, tag):
            # Evaluate the closed boolean/string subset used by these actual
            # YAML conditions, with successful prerequisites; no script runs.
            expression = condition(node).removeprefix("${{").removesuffix("}}").strip()
            values = {"matrix.target": target, "github.event_name": event, "env.RELEASE_TAG": tag,
                      "needs.alpha-tag.outputs.tag": tag if event != "push" else "",
                      "needs.alpha-tag.result": "success" if tag and event != "push" else "skipped",
                      "needs.build-tools.result": "success", "needs.unica-ci.result": "success",
                      "needs.rust-os.result": "success", "needs.package-thin.result": "success",
                      "needs.publish-release-assets.result": "success",
                      "needs.unica-ci.outputs.release_required": "true", "needs.unica-ci.outputs.ci_changed": "true"}
            for key, value in values.items():
                expression = expression.replace(key, repr(value))
            expression = expression.replace("&&", " and ").replace("||", " or ")
            return bool(eval(expression, {"__builtins__": {}, "always": lambda: True}))

        for target in ("linux-x64", "darwin-arm64", "win-x64"):
            for event, tag in (("workflow_dispatch", ""), ("schedule", ""), ("push", "v0.15.0"),
                               ("schedule", "v0.15.0-alpha.1"), ("workflow_dispatch", "v0.15.0-alpha.1")):
                expected = target == "linux-x64" or (target == "darwin-arm64" and event != "push") or bool(tag)
                with self.subTest(target=target, event=event, tag=tag):
                    self.assertEqual(allowed(upload, target, event, tag), expected)
                    self.assertTrue(allowed(build, target, event, tag))
                    if not tag:
                        for name in ("publish-release-assets",):
                            self.assertFalse(allowed(job(self.release, name), target, event, tag))
                    else:
                        self.assertTrue(allowed(job(self.release, "publish-release-assets"), target, event, tag))
        self.assertEqual(upload["with"]["retention-days"], 1)
        self.assertEqual(upload["with"]["name"], "unica-runtime-${{ matrix.target }}")

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

        self.assertEqual(needs(thin), ["build-tools", "alpha-tag"])
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
            "darwin-arm64": "macos-15",
        }

        self.assertEqual(dict(targets(probe)), expected_targets)
        self.assertEqual(dict(targets(smoke)), expected_targets)
        step_named(probe, "Probe packaged bootstrap through the downloader")
        step_named(smoke, "Smoke packaged bootstrap against published runtime")
        self.assertIn("scripts/ci/smoke-unica-bootstrap.py", script(smoke))
        self.assertIn(" --plugin-root .build/thin/plugins/unica", script(smoke))
        self.assertIn(' --target "${{ matrix.target }}"', script(smoke))
        self.assertEqual(needs(probe), ["package-thin", "alpha-tag"])
        self.assertEqual(needs(smoke), ["package-thin", "publish-release-assets", "alpha-tag"])
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
        self.assertEqual(needs(publish), ["build-tools", "alpha-tag"])
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
            [("unica-runtime-*", "dist/runtime"), ("unica-metadata-*", "dist/metadata"), ("unica-full-*", "dist/full")],
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
        # Токен есть только у заданий окружения `marketplace`, и кода тега там нет.
        for job_id, found in jobs(self.publish).items():
            if "UNICA_MARKETPLACE_TOKEN" not in "\n".join(strings(found)):
                continue
            with self.subTest(token_job=job_id):
                self.assertEqual(found["environment"], "marketplace")
                self.assertEqual(steps_using(found, "actions/checkout"), [step_named(found, "Check out the release channel rules")])
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
        text = "\n".join(script(job(self.publish, job_id)) for job_id in ("gate", "stage", "anchor", "verify-fresh-install", "verify-upgrade", "promote"))
        gate = job(self.publish, "gate")

        # Публикацию открывает только зелёная сборка этого же прогона по push тега.
        self.assertEqual(needs(gate), ["delivery-ci"])
        for clause in ("needs.delivery-ci.result == 'success'", "github.event_name == 'push'", "github.ref_type == 'tag'",
                       "github.repository == 'IngvarConsulting/unica'"):
            self.assertIn(clause, condition(gate))
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
        self.assertNotIn("source_run_id", on["workflow_dispatch"]["inputs"])
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
        # The payload is trusted only from this run's own tag: its manifest must
        # name the tag whose green build the run carries.
        self.assertIn('test "$GITHUB_REF_NAME" = "$RELEASE_TAG"', text)
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
                ("darwin-arm64", "macos-15"),
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
            self.assertEqual(needs(found), [*dependencies, "alpha-tag"], job_id)
            for dependency in dependencies:
                self.assertIn(f"needs.{dependency}.result == 'success'", condition(found))

        local_verifier = "python scripts/ci/verify-release-assets.py"
        self.assertIn(local_verifier, script(self.build))
        self.assertIn('--asset-dir ".build/runtime-assets/${{ matrix.target }}"', script(self.build))

        published = job(self.release, "verify-published-assets")
        published_lifecycle = (
            'gh release download "$RELEASE_TAG" --pattern \'unica-runtime-*\' --dir published',
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

        self.assertEqual(normalized(release["with"]["prerelease"]), "${{ contains(env.RELEASE_TAG, '-') }}")

    def test_publication_asks_first_which_channel_the_release_reaches(self) -> None:
        gate = job(self.publish, "gate")

        self.assertEqual(gate["outputs"], {"channel": "${{ steps.decide.outputs.channel }}"})
        self.assertIn(
            'python3 rules/scripts/ci/release-channel.py channel "$GITHUB_REF_NAME"',
            step_named(gate, "Decide which channel this release reaches")["run"],
        )

    def test_every_publishing_stage_waits_for_that_answer(self) -> None:
        # Достаточно загейтить первую стадию: остальные ждут её через `needs`.
        # Предвыпуск не `-rc.N` существует для замеров и не публикуется.
        stage = job(self.publish, "stage")

        self.assertEqual(needs(stage), ["gate"])
        self.assertEqual(
            condition(stage),
            "${{ !cancelled() && needs.gate.result == 'success' && (needs.gate.outputs.channel == 'stable' || needs.gate.outputs.channel == 'next') }}",
        )

    def test_a_skipped_ancestor_does_not_silently_stop_the_publication(self) -> None:
        # На теге пропущены `alpha-tag`, проба и `p0-release-proof`. Без статусной
        # функции GitHub подставляет `success()`, и выпуск молча не доходил бы до
        # каталога при зелёном прогоне.
        required = {
            "gate": ["needs.delivery-ci.result == 'success'"],
            "stage": ["needs.gate.result == 'success'"],
            "anchor": ["needs.stage.result == 'success'"],
            "verify-fresh-install": ["needs.anchor.result == 'success'"],
            "verify-upgrade": ["needs.anchor.result == 'success'"],
            "promote": ["needs.anchor.result == 'success'", "needs.verify-fresh-install.result == 'success'",
                        "needs.verify-upgrade.result == 'success'"],
        }
        for job_id, clauses in required.items():
            with self.subTest(job_id=job_id):
                gate = normalized(condition(job(self.publish, job_id)))
                self.assertIn("!cancelled()", gate)
                for clause in clauses:
                    self.assertIn(clause, gate)
        # То же для всех заданий, чей предок может быть пропущен.
        skippable = {"alpha-tag", "windows-clippy", "probe-thin-bootstrap", "p0-release-proof", "release-assessment"}
        for job_id, found in jobs(self.release).items():
            ancestors, frontier = set(), set(needs(found))
            while frontier:
                current = frontier.pop()
                if current not in ancestors:
                    ancestors.add(current)
                    frontier |= set(needs(job(self.release, current)))
            if ancestors & skippable:
                with self.subTest(job_id=job_id):
                    gate = condition(found)
                    self.assertTrue("!cancelled()" in gate or "always()" in gate, gate)

    def test_channel_rules_come_from_main_without_credentials(self) -> None:
        # Код тега не исполняется рядом с токеном маркетплейса: правила канала
        # берутся из main, и checkout не оставляет токена.
        for job_id in ("gate", "stage", "anchor", "verify-fresh-install", "verify-upgrade", "promote"):
            with self.subTest(job_id=job_id):
                checkout = step_named(job(self.publish, job_id), "Check out the release channel rules")
                self.assertTrue(checkout["uses"].startswith("actions/checkout@"))
                self.assertEqual(
                    checkout["with"],
                    {
                        "ref": "main",
                        "path": "rules",
                        "persist-credentials": False,
                        "sparse-checkout": "scripts/ci/release-channel.py",
                        "sparse-checkout-cone-mode": False,
                    },
                )

if __name__ == "__main__":
    unittest.main()
