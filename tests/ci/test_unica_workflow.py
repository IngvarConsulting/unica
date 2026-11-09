"""Стражи workflow: поведение, а не текст YAML.

Проверяется то, что решает GitHub: триггеры, связи заданий (`needs`), условия,
передаваемые артефакты, окружения и права. Шаг находится по `id` или действию,
а не по имени. Условия вычисляются по таблице исходов, а не сравниваются
строкой; короткие скрипты шагов исполняются в bash с подменой внешних команд.
Имена шагов, тексты команд и их аргументы не закреплены: переименование или
другая запись того же поведения стража не красит.

Текстом остаются две вещи: комментарий версии у пина действия (его читает
Dependabot, а не GitHub) и маркеры, которых в файле быть не должно вовсе.
"""

from __future__ import annotations

import fnmatch
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from collections.abc import Iterator
from pathlib import Path

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import ci_rust_lanes as lanes  # noqa: E402


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
RELEASE_WORKFLOW = WORKFLOWS_DIR / "release.yml"
CI_WORKFLOW = WORKFLOWS_DIR / "ci.yml"
CLIPPY_SCRIPT = REPO_ROOT / "scripts" / "ci" / "clippy.sh"
NIGHTLY_WORKFLOW = WORKFLOWS_DIR / "nightly.yml"
PAGES_WORKFLOW = WORKFLOWS_DIR / "release.yml"
PUBLISH_WORKFLOW = WORKFLOWS_DIR / "release.yml"
PUBLISH_JOBS = ("gate", "stage", "anchor", "verify-fresh-install", "verify-upgrade", "promote")


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


def ancestors(workflow: dict, job_id: str) -> set[str]:
    """Все задания, которых это задание ждёт прямо или через других."""
    found, frontier = set(), set(needs(job(workflow, job_id)))
    while frontier:
        current = frontier.pop()
        if current not in found:
            found.add(current)
            frontier |= set(needs(job(workflow, current)))
    return found


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


def step_by_id(node: dict, step_id: str) -> dict:
    for step in steps(node):
        if step.get("id") == step_id:
            return step
    raise AssertionError(f"нет шага с id {step_id!r}")


def uploads(node: dict) -> list[dict]:
    return [step.get("with") or {} for step in steps_using(node, "actions/upload-artifact")]


def downloads(node: dict) -> list[dict]:
    return [step.get("with") or {} for step in steps_using(node, "actions/download-artifact")]


def upload_step(node: dict, name: str) -> dict:
    """Шаг выгрузки артефакта — по имени артефакта, которое читают потребители."""
    for step in steps_using(node, "actions/upload-artifact"):
        if (step.get("with") or {}).get("name") == name:
            return step
    raise AssertionError(f"нет выгрузки артефакта {name!r}")


def matrix_include(node: dict) -> list[dict]:
    return list((node.get("strategy") or {}).get("matrix", {}).get("include", []))


def targets(node: dict) -> list[tuple[str, str]]:
    return [(entry["target"], entry["runner"]) for entry in matrix_include(node)]


_CONTEXT = re.compile(r"[A-Za-z_][\w-]*(?:\.[\w-]+)+")
_FUNCTIONS = {
    "always": lambda: True,
    "cancelled": lambda: False,
    "success": lambda: True,
    "contains": lambda haystack, needle: str(needle) in str(haystack),
    "startsWith": lambda value, prefix: str(value).startswith(str(prefix)),
    "fromJSON": json.loads,
    "format": lambda template, *args: re.sub(r"\{(\d+)\}", lambda match: str(args[int(match.group(1))]), template),
}


def evaluate(expression: object, values: dict[str, object]) -> object:
    """Вычисляет выражение GitHub на заданных значениях контекстов.

    Хватает подмножества, которым пишутся условия этих workflow: сравнения,
    `&&`, `||`, `!`, строки и функции из `_FUNCTIONS`. Контекст, которого нет в
    `values`, — ошибка: тест обязан назвать каждое значение, от которого
    зависит исход.
    """
    text = normalized(expression)
    if text.startswith("${{") and text.endswith("}}"):
        text = text[3:-2]
    parts = re.split(r"('(?:[^']|'')*')", text)
    for index in range(0, len(parts), 2):
        part = parts[index].replace("&&", " and ").replace("||", " or ")
        part = re.sub(r"!(?!=)", " not ", part)
        parts[index] = _CONTEXT.sub(lambda match: repr(values[match.group(0)]), part)
    return eval("".join(parts), {"__builtins__": {}}, dict(_FUNCTIONS))  # noqa: S307 — закрытое подмножество


def expand_braces(pattern: str) -> list[str]:
    """`{a,b}-x` → `a-x`, `b-x`: так `download-artifact` читает шаблон."""
    match = re.search(r"\{([^{}]*)\}", pattern)
    if not match:
        return [pattern]
    return [
        expanded
        for option in match.group(1).split(",")
        for expanded in expand_braces(pattern[: match.start()] + option + pattern[match.end():])
    ]


def matrix_values(node: dict, profile: str = "large") -> dict[str, list[str]]:
    matrix = (node.get("strategy") or {}).get("matrix") or {}
    found: dict[str, list[str]] = {}
    for entry in matrix.get("include", []):
        for key, value in entry.items():
            found.setdefault(key, []).append(str(value))
    for key, value in matrix.items():
        if key == "include":
            continue
        found[key] = list(evaluate(value, {"inputs.profile": profile}) if isinstance(value, str) else value)
    return found


def artifact_names(node: dict, name: str) -> list[str]:
    """Имя артефакта для каждой строки матрицы задания."""
    names = [name]
    for key, options in matrix_values(node).items():
        token = "${{ matrix.%s }}" % key
        if any(token in found for found in names):
            names = [found.replace(token, option) for found in names for option in options]
    return names


def bash(text: str, *, cwd: Path, env: dict[str, str] | None = None, prelude: str = "") -> subprocess.CompletedProcess:
    """Исполняет скрипт шага так, как GitHub: `bash --noprofile --norc -eo pipefail`."""
    assert "${{" not in text, "шаг с выражением в теле исполняется только после подстановки"
    return subprocess.run(
        ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", prelude + text],
        cwd=cwd, env={**os.environ, **(env or {})}, capture_output=True, text=True, check=False,
    )


def python_prelude() -> str:
    """`python`/`python3` в шаге — это интерпретатор теста, а не алиас Store."""
    executable = Path(sys.executable).as_posix()
    return f'python() {{ "{executable}" "$@"; }}\npython3() {{ "{executable}" "$@"; }}\n'


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


def assert_env_consumed(test: unittest.TestCase, step: dict, name: str, value: str) -> None:
    """Значение приходит в шаг через `env` и шаг его читает: привязка не мёртвая."""
    test.assertEqual(normalized(step["env"][name]), value)
    test.assertRegex(step["run"], rf"\${{?{name}\b")


# Исходы, при которых сборка и выкладка идут: всё зелёное, alpha не выпускается.
GREEN = {
    "needs.unica-ci.result": "success",
    "needs.rust-os.result": "success",
    "needs.alpha-tag.result": "skipped",
    "needs.alpha-tag.outputs.tag": "",
    "needs.build-tools.result": "success",
    "needs.package-thin.result": "success",
    "needs.release-assessment.result": "success",
    "needs.publish-release-assets.result": "success",
    "needs.unica-ci.outputs.release_required": "true",
    "needs.unica-ci.outputs.ci_changed": "false",
    "needs.unica-ci.outputs.assessment_required": "true",
    "github.event_name": "workflow_dispatch",
}


class EvaluateTests(unittest.TestCase):
    """Вычислитель условий сам не должен врать: иначе таблицы ниже пусты."""

    def test_operators_follow_github_semantics(self) -> None:
        values = {"a.b": "x", "c.d": ""}
        cases = (
            ("${{ a.b == 'x' && c.d == '' }}", True),
            ("${{ a.b != 'x' || c.d != '' }}", False),
            ("${{ !cancelled() && !contains(a.b, 'y') }}", True),
            ("${{ c.d || 'fallback' }}", "fallback"),
            ("${{ startsWith(a.b, 'x') && 'yes' || 'no' }}", "yes"),
            ("${{ fromJSON(a.b == 'x' && '[\"m\"]' || '[]') }}", ["m"]),
        )
        for expression, expected in cases:
            with self.subTest(expression=expression):
                self.assertEqual(evaluate(expression, values), expected)

    def test_an_unnamed_context_is_an_error(self) -> None:
        with self.assertRaises(KeyError):
            evaluate("${{ inputs.profile == 'large' }}", {})

    def test_braces_expand_like_download_patterns(self) -> None:
        self.assertEqual(expand_braces("{plan,results}-{a,b}"), ["plan-a", "plan-b", "results-a", "results-b"])


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


class PersistentRunnerCacheTests(unittest.TestCase):
    def test_cache_and_retries_follow_the_runner_switch(self) -> None:
        gate = job(load(CI_WORKFLOW), "unica-ci")
        park = step_by_id(gate, "park-target")
        local = step_by_id(gate, "local-target")
        remote = steps_using(gate, "Swatinem/rust-cache")[0]
        for event, attempt, enabled, environment in (
            ("merge_group", 1, "true", "self-hosted"),
            ("merge_group", 2, "true", "self-hosted"),
            ("merge_group", 1, "false", "github-hosted"),
            ("merge_group", 2, "false", "github-hosted"),
            ("merge_group", 1, "", "github-hosted"),
            ("pull_request", 1, "true", "github-hosted"),
        ):
            with self.subTest(event=event, attempt=attempt, enabled=enabled):
                labels = evaluate(gate["runs-on"], {
                    "github.event_name": event, "github.run_attempt": attempt,
                    "vars.UNICA_SELF_HOSTED": enabled,
                })
                self.assertEqual("self-hosted" in labels, environment == "self-hosted")
                for kept in ("", "true"):
                    values = {"runner.environment": environment, "env.KEPT": kept}
                    self.assertEqual(evaluate(park["if"], values), environment == "self-hosted" and kept != "true")
                    self.assertEqual(evaluate(local["if"], values), environment == "self-hosted" and kept != "true")
                    self.assertEqual(evaluate(remote["if"], values), environment == "github-hosted" and kept != "true")
        order = steps(gate)
        self.assertLess(order.index(park), order.index(steps_using(gate, "actions/checkout")[0]))
        self.assertLess(order.index(steps_using(gate, "actions/checkout")[0]), order.index(local))
        self.assertLess(order.index(local), order.index(step_by_id(gate, "clippy")))

    def run_cache_step(self, workspace: Path, step_id: str, *, check: bool = True):
        step = step_by_id(job(load(CI_WORKFLOW), "unica-ci"), step_id)
        return subprocess.run(["bash", "-e", "-o", "pipefail", "-c", step["run"]],
                              cwd=workspace, env={**os.environ, "GITHUB_WORKSPACE": str(workspace)},
                              check=check, capture_output=True)

    @unittest.skipIf(os.name == "nt", "PVE shell steps are used only on Linux")
    def test_checkout_cleans_sources_but_preserves_each_runner_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            targets = []
            for name in ("runner one", "runner two"):
                workspace = Path(temporary) / name / "unica"
                workspace.mkdir(parents=True)
                def git(*args):
                    return subprocess.run(["git", *args], cwd=workspace, check=True, capture_output=True)
                git("init", "-q")
                (workspace / ".gitignore").write_text((REPO_ROOT / ".gitignore").read_text(encoding="utf-8"), encoding="utf-8")
                (workspace / "source").write_text("tracked", encoding="utf-8")
                git("add", ".")
                git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "fixture")
                self.run_cache_step(workspace, "park-target")
                self.run_cache_step(workspace, "local-target")
                target = workspace / "target"
                target.mkdir()
                self.assertEqual(git("ls-files", "--others", "--exclude-standard").stdout, b"")
                targets.append(target.resolve())
                (target / "artifact").write_text(name, encoding="utf-8")
                (workspace / "untracked").write_text("old job", encoding="utf-8")
                (workspace / "source").write_text("dirty", encoding="utf-8")
                self.run_cache_step(workspace, "park-target")
                git("clean", "-ffdx")
                git("reset", "--hard", "HEAD")
                self.assertFalse(target.exists())
                self.assertFalse((workspace / "untracked").exists())
                self.assertEqual((workspace / "source").read_text(encoding="utf-8"), "tracked")
                # A retry after interruption must keep the parked directory.
                self.run_cache_step(workspace, "park-target")
                self.run_cache_step(workspace, "local-target")
                self.assertFalse(target.is_symlink())
                self.assertEqual(target.resolve(), workspace.resolve() / "target")
                self.assertEqual((target / "artifact").read_text(encoding="utf-8"), name)
            self.assertNotEqual(*targets)

    @unittest.skipIf(os.name == "nt", "PVE shell steps are used only on Linux")
    def test_cache_conflicts_and_invalid_parks_preserve_existing_data(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary) / "unica"
            workspace.mkdir()
            target = workspace / "target"
            target.mkdir()
            (target / "current").write_text("current", encoding="utf-8")
            park = workspace.parent / ".unica-ci-target"
            park.mkdir()
            (park / "saved").write_text("saved", encoding="utf-8")
            for step in ("park-target", "local-target"):
                self.assertNotEqual(self.run_cache_step(workspace, step, check=False).returncode, 0)
                self.assertEqual(sorted(p.name for p in target.iterdir()), ["current"])
                self.assertEqual(sorted(p.name for p in park.iterdir()), ["saved"])
            shutil.rmtree(park)
            park.write_text("not a directory", encoding="utf-8")
            self.assertNotEqual(self.run_cache_step(workspace, "local-target", check=False).returncode, 0)
            self.assertEqual(park.read_text(encoding="utf-8"), "not a directory")
            park.unlink()
            park.symlink_to(workspace.parent / "missing", target_is_directory=True)
            for step in ("park-target", "local-target"):
                self.assertNotEqual(self.run_cache_step(workspace, step, check=False).returncode, 0)
                self.assertTrue(park.is_symlink())


class UnicaWorkflowGuardrailTests(unittest.TestCase):
    def setUp(self) -> None:
        self.release = load(RELEASE_WORKFLOW)
        self.nightly = load(NIGHTLY_WORKFLOW)
        self.pages = load(PAGES_WORKFLOW)
        self.publish = load(PUBLISH_WORKFLOW)

    def test_source_gate_checks_the_full_rust_and_python_workspace(self) -> None:
        gate = job(self.release, "unica-ci")
        # Clippy покрывает все цели и все признаки, предупреждение — отказ.
        clippy = CLIPPY_SCRIPT.read_text(encoding="utf-8")
        for flag in ("--workspace", "--all-targets", "--all-features", "-D warnings"):
            self.assertIn(flag, clippy)
        # Исполнение тестов остаётся у runner: ни одна джоба не зовёт их мимо шва.
        for job_id, found in jobs(self.release).items():
            for step in steps(found):
                with self.subTest(job=job_id, step=step.get("id")):
                    self.assertNotIn("cargo test", step.get("run", ""))
                    self.assertNotIn("unittest discover", step.get("run", ""))
        for step_id in ("guards-compile", "guards-version", "guards-fmt", "rust-run", "python"):
            step_by_id(gate, step_id)

    def test_label_changes_cannot_replace_the_stable_pull_request_gate(self) -> None:
        ci = load(CI_WORKFLOW)
        on = triggers(ci)
        ci_gate = job(ci, "unica-ci")
        gate = job(self.release, "unica-ci")

        # Изменение метки не должно создавать новый suite этого workflow:
        # ruleset смотрит на последний suite головы PR и перестаёт видеть
        # обязательный `Unica CI`, если тот suite состоит из skipped-джоб.
        self.assertEqual(set(on["pull_request"]["types"]), {"opened", "synchronize", "reopened", "ready_for_review", "milestoned"})
        self.assertNotIn("paths", on["pull_request"])
        self.assertNotIn("label ignored", "\n".join(strings(self.release)))
        # Имена — обязательные проверки ruleset.
        self.assertEqual(ci_gate["name"], "Unica CI")
        self.assertEqual(gate["name"], "Unica CI")
        # Пропуск тестов без допуска ruleset считает пройденным; вливание блокирует
        # отдельная обязательная `PR metadata`, см. test_pr_metadata_workflow.
        self.assertEqual(needs(ci_gate), ["admission"])
        for admitted in ("true", "false"):
            with self.subTest(admitted=admitted):
                self.assertEqual(evaluate(ci_gate["if"], {"needs.admission.outputs.admitted": admitted}), admitted == "true")
        self.assertNotIn("if", gate)
        self.assertEqual(needs(gate), [])
        # Оценка — последний шаг и идёт всегда; она видит исходы всех шагов.
        evaluate_step = step_by_id(gate, "gate")
        self.assertIs(steps(gate)[-1], evaluate_step)
        self.assertEqual(condition(evaluate_step), "always()")
        self.assertEqual(evaluate_step["env"]["GATE_SCOPE"], "validation")
        self.assertEqual(normalized(evaluate_step["env"]["STEPS_JSON"]), "${{ toJSON(steps) }}")
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
        self.assertEqual(steps(delivery)[-1]["env"]["GATE_SCOPE"], "delivery")
        # Итог поставки ждёт каждое задание сборки и выкладки до публикации.
        self.assertEqual(set(needs(delivery)), {
            "unica-ci", "rust-os", "build-tools", "package-thin", "probe-thin-bootstrap", "release-assessment",
            "publish-release-assets", "smoke-thin-plugin", "verify-published-assets", "p0-release-proof", "alpha-tag",
        })
        self.assertEqual(condition(delivery), "${{ always() }}")

    def test_build_workflow_does_not_subscribe_to_label_or_milestone_changes(self) -> None:
        # Проверяем все входы сборочного workflow: безопасный pull_request
        # не исключает параллельный pull_request_target с metadata types.
        self.assertEqual(metadata_build_triggers(self.release), {})

    def test_p0_dry_release_proof_is_read_only_and_aggregated(self) -> None:
        proof = job(self.release, "p0-release-proof")

        self.assertEqual(set(needs(proof)), {"build-tools", "package-thin", "release-assessment", "alpha-tag"})
        # Proof идёт до публикации: на push тега он не нужен, на остальных — после
        # зелёной сборки, пакета и оценки.
        self.assertTrue(evaluate(proof["if"], GREEN))
        self.assertFalse(evaluate(proof["if"], {**GREEN, "github.event_name": "push"}))
        for red in ("needs.build-tools.result", "needs.package-thin.result", "needs.release-assessment.result"):
            with self.subTest(red=red):
                self.assertFalse(evaluate(proof["if"], {**GREEN, red: "failure"}))
        # Идентичность пакета включает бит исполнения, а
        # actions/download-artifact режимы не сохраняет: пакет для сверки
        # приходит через jobs API, которому нужно `actions: read`.
        self.assertEqual(proof["permissions"], {"contents": "read", "actions": "read"})
        self.assertIn("unica-thin-marketplace", script(proof))
        for found in downloads(proof):
            self.assertNotEqual(found.get("name"), "unica-thin-marketplace")
        self.assertEqual(steps_using(proof, "softprops/action-gh-release"), [])
        self.assertNotIn("environment", proof)
        # Read-only-ность proof живёт здесь, в устройстве джобы, а не в
        # самоотчётных полях JSON: нет прав на запись, нет шагов
        # публикации, нет ни тега, ни push, ни бампа версии.
        for marker in ("git tag", "git push", "gh release", "bump-version", "secrets."):
            self.assertNotIn(marker, "\n".join(strings(proof)))
        self.assertIn("p0-release-proof", needs(job(self.release, "delivery-ci")))

    def test_wire_probes_embed_the_matrix_target_in_their_evidence(self) -> None:
        build = job(self.release, "build-tools")
        for step_id in ("wire-native", "wire-compat"):
            with self.subTest(step=step_id):
                assert_env_consumed(self, step_by_id(build, step_id), "TARGET", "${{ matrix.target }}")
        # Доказательства каждой цели уходят своим артефактом.
        self.assertIn("unica-wire-${{ matrix.target }}", [upload.get("name") for upload in uploads(build)])

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
        self.assertEqual(set(scope["env"]), {"RUNNERS", "RUN_LINE"})
        assert_env_consumed(self, scope, "RUNNERS", "${{ steps.runners.outputs.runners }}")
        # Джоба ОС идёт параллельно и маршрут считает своим шагом.
        step_by_id(job(self.release, "rust-os"), "classify")

    def test_every_classifier_call_gets_the_resolved_line(self) -> None:
        """Документационный пропуск пишет пометку с линией: пустая линия —
        отказ классификатора, и PR из одной прозы стал бы красным."""
        calls = 0
        for job_id, found in jobs(self.release).items():
            for step in steps(found):
                if step.get("id") != "classify":
                    continue
                calls += 1
                with self.subTest(job=job_id):
                    assert_env_consumed(self, step, "RUN_LINE", "${{ steps.line.outputs.line }}")
                    order = [other.get("id") for other in steps(found)]
                    self.assertLess(order.index("line"), order.index("classify"))
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
        for found in (gate, other):
            build, run = step_by_id(found, "rust-build"), step_by_id(found, "rust-run")
            with self.subTest(job=found["name"]):
                for step in (build, run):
                    self.assertIn("rust_required == 'true'", condition(step))
                self.assertIn("!cancelled()", condition(run))
                order = steps(found)
                self.assertLess(order.index(build), order.index(run))
                # Планы выгружаются до тестов: красный прогон всё равно знает план.
                plan_upload = next(step for step in steps_using(found, "actions/upload-artifact") if step["with"]["name"].startswith("plan-"))
                self.assertLess(order.index(build), order.index(plan_upload))
                self.assertLess(order.index(plan_upload), order.index(run))
        verify = step_by_id(gate, "verify")
        order = steps(gate)
        self.assertLess(order.index(step_by_id(gate, "os-jobs")), order.index(verify))
        step_by_id(gate, "guards-fmt")
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
        self.assertNotIn("continue-on-error", clippy)
        self.assertLess(order.index(step_by_id(gate, "rust-build")), order.index(start))
        self.assertLess(order.index(start), order.index(step_by_id(gate, "rust-run")))
        self.assertLess(order.index(step_by_id(gate, "rust-run")), order.index(clippy))
        self.assertLess(order.index(clippy), order.index(step_by_id(gate, "python")))
        # Джоба другой ОС линтует везде, где идёт.
        os_clippy = step_by_id(other, "clippy")
        self.assertNotIn("if", other)
        # Матрица — один macOS; Windows только в `large`.
        for profile, runners in (("main", ["macos-15"]), ("large", ["macos-15", "windows-latest"])):
            with self.subTest(profile=profile):
                self.assertEqual(matrix_values(other, profile)["runner"], runners)
        # Ни одна ОС не рекомендательная: красная джоба красит прогон.
        self.assertNotIn("continue-on-error", other)
        self.assertIn("rust_required == 'true'", condition(os_clippy))
        for flag in ("--workspace", "--all-targets", "--all-features", "-D warnings"):
            self.assertIn(flag, os_clippy["run"])
        self.assertNotIn("continue-on-error", os_clippy)

    def test_os_job_lints_first_and_a_red_clippy_still_runs_the_tests(self) -> None:
        """Clippy джобы ОС — до сборки тестов; его отказ не отменяет сборку и прогон."""
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
        """Ошибка Clippy, видимая только на Windows, не проходит кнопку `main`."""
        lint = job(self.release, "windows-clippy")
        self.assertEqual(lint["runs-on"], "windows-latest")
        self.assertEqual(lint["timeout-minutes"], 30)
        for profile in ("main", "large"):
            with self.subTest(profile=profile):
                self.assertEqual(evaluate(lint["if"], {"inputs.profile": profile}), profile == "main")
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
        # Маршрут считается для своей ОС.
        classify = step_by_id(lint, "classify")
        self.assertIn(lint["runs-on"], classify["run"])
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
        for flag in ("--workspace", "--all-targets", "--all-features", "-D warnings"):
            self.assertIn(flag, clippy["run"])
        self.assertNotIn("continue-on-error", clippy)
        for step in (*toolchain, cache[0], clippy):
            for required in ("true", "false"):
                with self.subTest(step=step.get("id") or action(step), required=required):
                    self.assertEqual(
                        evaluate(step["if"], {"steps.classify.outputs.rust_required": required}), required == "true",
                    )
        # Гейт ждёт исход джобы целиком: без Rust она краснеет.
        required = step_by_id(lint, "rust-required")
        for rust in ("true", "false"):
            with self.subTest(rust_required=rust):
                self.assertEqual(evaluate(required["if"], {"steps.classify.outputs.rust_required": rust}), rust != "true")
        with tempfile.TemporaryDirectory() as tmp:
            self.assertNotEqual(bash(required["run"], cwd=Path(tmp)).returncode, 0)
        order = steps(lint)
        self.assertLess(order.index(classify), order.index(required))
        self.assertLess(order.index(required), order.index(toolchain[0]))
        self.assertLess(order.index(toolchain[0]), order.index(cache[0]))
        self.assertLess(order.index(cache[0]), order.index(clippy))
        # `Unica CI` ждёт её по jobs API под её именем и без успеха краснеет.
        gate = job(self.release, "unica-ci")
        wait = step_by_id(gate, "windows-clippy")
        for profile in ("main", "large"):
            for rust in ("true", "false"):
                with self.subTest(profile=profile, rust_required=rust):
                    self.assertEqual(
                        evaluate(wait["if"], {"steps.classify.outputs.rust_required": rust, "inputs.profile": profile}),
                        evaluate(lint["if"], {"inputs.profile": profile}) and rust == "true",
                    )
        self.assertIn(lint["name"], wait["run"])
        self.assertIn("--require-success", wait["run"])
        self.assertEqual(wait["env"]["GH_TOKEN"], "${{ github.token }}")
        self.assertLess(steps(gate).index(wait), steps(gate).index(step_by_id(gate, "gate")))
        self.assertIn("windows-clippy", load_gate_module().STEP_GROUPS["lint-rust-windows"])

    def test_package_contour_and_pr_smoke_do_not_publish_release_assets(self) -> None:
        build = job(self.release, "build-tools")
        probe = job(self.release, "probe-thin-bootstrap")
        publish = job(self.release, "publish-release-assets")

        # Сборка идёт, когда маршрут требует выпуска или CI, и только после
        # зелёных ворот обеих ОС.
        self.assertTrue(evaluate(build["if"], GREEN))
        self.assertTrue(evaluate(build["if"], {**GREEN, "needs.unica-ci.outputs.release_required": "false",
                                               "needs.unica-ci.outputs.ci_changed": "true"}))
        self.assertFalse(evaluate(build["if"], {**GREEN, "needs.unica-ci.outputs.release_required": "false"}))
        for red in ("needs.unica-ci.result", "needs.rust-os.result"):
            with self.subTest(red=red):
                self.assertFalse(evaluate(build["if"], {**GREEN, red: "failure"}))
        # Неудача alpha не собирает непомеченное дерево как будто всё прошло.
        self.assertTrue(evaluate(build["if"], {**GREEN, "needs.alpha-tag.result": "success"}))
        self.assertFalse(evaluate(build["if"], {**GREEN, "needs.alpha-tag.result": "failure"}))
        # Проба ждёт отказа скачивания: у выпуска и alpha ассеты уже есть.
        self.assertTrue(evaluate(probe["if"], GREEN))
        self.assertFalse(evaluate(probe["if"], {**GREEN, "github.event_name": "push"}))
        self.assertFalse(evaluate(probe["if"], {**GREEN, "needs.alpha-tag.outputs.tag": "v1.0.0-alpha.1"}))
        # Публикует только тег или выпущенный alpha, кнопка — нет.
        self.assertFalse(evaluate(publish["if"], GREEN))
        self.assertTrue(evaluate(publish["if"], {**GREEN, "github.event_name": "push"}))
        self.assertTrue(evaluate(publish["if"], {**GREEN, "needs.alpha-tag.outputs.tag": "v1.0.0-alpha.1"}))

    def test_release_workflow_runs_by_tag_button_and_the_nightly_call(self) -> None:
        """Ворота pull request и очереди живут в `ci.yml`, здесь выпуск, кнопка и вызов ночи."""
        on = triggers(self.release)

        self.assertEqual(set(on), {"workflow_dispatch", "workflow_call", "push"})
        # Теги alpha ставит ключ развёртывания, а собирает их ночь: прогона по тегу у alpha нет.
        self.assertEqual(on["push"], {"tags": ["v*", "!v*-alpha.*"]})
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
            with self.subTest(step=step.get("id") or step.get("uses") or index):
                gate_condition = condition(step)
                # Единственное исключение — база следующих прогонов: её
                # публикует только полностью зелёный прогон.
                if (step.get("with") or {}).get("name") == "ci-rust-inventory-${{ env.GATE_PROFILE }}":
                    self.assertIn("success()", gate_condition)
                    continue
                self.assertTrue("!cancelled()" in gate_condition or "always()" in gate_condition, gate_condition)
        self.assertIs(job(self.release, "rust-os")["strategy"]["fail-fast"], False)
        build = job(self.release, "build-tools")
        self.assertEqual(needs(build), ["unica-ci", "rust-os", "alpha-tag"])
        # Результаты выгружаются и при красном прогоне.
        for found in (gate, job(self.release, "rust-os")):
            results = next(step for step in steps_using(found, "actions/upload-artifact") if step["with"]["name"].startswith("results-"))
            self.assertEqual(condition(results), "always()")

    def test_one_red_python_suite_does_not_stop_the_others(self) -> None:
        """Шаг Python исполняется: каждый набор доходит, отказ одного красит шаг."""
        python = step_by_id(job(self.release, "unica-ci"), "python")
        matrix = [{"suite": name, "lane": "small", "slug": name} for name in ("first", "broken", "last")]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for entry in matrix:
                (root / "report" / f"plan-python-{entry['slug']}").mkdir(parents=True)
            (root / ".build").mkdir()
            jq = root / "jq.py"
            jq.write_text(
                "import json, sys\n"
                "args = [a for a in sys.argv[1:] if not a.startswith('-')]\n"
                "data = json.load(sys.stdin)\n"
                "if args[0] == '.[]':\n"
                "    print('\\n'.join(json.dumps(item) for item in data))\n"
                "else:\n"
                "    print(data[args[0].lstrip('.')])\n",
                encoding="utf-8",
            )
            executable = Path(sys.executable).as_posix()
            prelude = (
                f'jq() {{ "{executable}" "{jq.as_posix()}" "$@"; }}\n'
                # Подменённый раннер помнит каждый вызов и падает на наборе `broken`.
                'python() { echo "$*" >> calls.txt; case " $* " in *" broken "*) return 1;; esac; }\n'
            )
            env = {"REPORT": "report", "PYTHON_MATRIX": json.dumps(matrix), "RUN_LINE": "main", "DOCS_ONLY": "false",
                   "GATE_PROFILE": "pr"}
            result = bash(python["run"], cwd=root, env=env, prelude=prelude)
            self.assertNotEqual(result.returncode, 0, result.stdout)
            calls = (root / "calls.txt").read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(calls), len(matrix))
            for entry in matrix:
                with self.subTest(suite=entry["suite"]):
                    self.assertTrue(any(f" {entry['suite']} " in f" {call} " for call in calls))
            self.assertIn("broken", result.stdout)

    def test_rust_cache_is_shared_without_a_lint_only_writer(self) -> None:
        for job_id in ("unica-ci", "rust-os"):
            found = job(self.release, job_id)
            cache = steps_using(found, "Swatinem/rust-cache")
            self.assertEqual(len(cache), 1)
            assert_pinned(self, RELEASE_WORKFLOW, cache[0], "v2")
            # Один ключ зависимостей на ОС: архивы, продукт и Clippy.
            self.assertEqual(cache[0]["with"]["shared-key"], "ci-rust-ordinary")
            # Пишут кэш ветки; тег и очередь только читают.
            for ref_type in ("branch", "tag"):
                with self.subTest(job=job_id, ref_type=ref_type):
                    self.assertEqual(evaluate(cache[0]["with"]["save-if"], {"github.ref_type": ref_type}), ref_type == "branch")
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
            nextest = next(step for step in steps_using(found, "./.github/actions/install-nextest"))
            self.assertLess(order.index(nextest), order.index(cache[0]))
        # CI sets no `CARGO_TARGET_DIR`: the archive goes to `<root>/target`.
        with mock.patch.dict(os.environ):
            os.environ.pop("CARGO_TARGET_DIR", None)
            for lane in ("ordinary", "receipt", "vendor"):
                archive = lanes.archive_command(lane, Path("tests.tar.zst"))
                self.assertEqual(Path(archive[archive.index("--target-dir") + 1]), lanes.ROOT / "target")
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
        gate = job(self.release, "unica-ci")
        installed = {tool.split("@", 1)[0] for step in steps_using(gate, "taiki-e/install-action")
                     for tool in step["with"]["tool"].split(",")}
        self.assertEqual(installed, {"zizmor", "clippy-sarif", "sarif-fmt"})
        self.assertEqual(len(steps_using(gate, "./.github/actions/install-nextest")), 1)

    def test_release_assessment_uses_affected_mechanism_contour(self) -> None:
        assessment = job(self.release, "release-assessment")

        self.assertEqual(needs(assessment), ["unica-ci", "build-tools", "alpha-tag"])
        self.assertTrue(evaluate(assessment["if"], GREEN))
        self.assertFalse(evaluate(assessment["if"], {**GREEN, "needs.unica-ci.outputs.assessment_required": "false"}))
        self.assertFalse(evaluate(assessment["if"], {**GREEN, "needs.build-tools.result": "failure"}))

    def test_release_assessment_uses_the_candidate_release_identity(self) -> None:
        assessment = job(self.release, "release-assessment")
        resolve = step_by_id(assessment, "release-tag")

        for tag in ("", "v1.2.3"):
            with self.subTest(tag=tag):
                self.assertEqual(evaluate(resolve["if"], {"env.RELEASE_TAG": tag}), tag == "")
        # Без тега версия кандидата берётся из манифеста пакета.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest = root / "plugins/unica/.codex-plugin/plugin.json"
            manifest.parent.mkdir(parents=True)
            manifest.write_text(json.dumps({"version": "1.2.3"}), encoding="utf-8")
            (root / "env").touch()
            result = bash(resolve["run"], cwd=root, env={"GITHUB_ENV": "env"}, prelude=python_prelude())
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((root / "env").read_text(encoding="utf-8").split(), ["RELEASE_TAG=v1.2.3"])
        self.assertTrue(any("$RELEASE_TAG" in text or "${RELEASE_TAG}" in text for text in scripts(assessment)
                            if text is not resolve["run"]))
        self.assertNotIn("RELEASE_REF", assessment["env"])

    def test_only_tag_pushes_and_a_released_alpha_enable_release_behavior(self) -> None:
        identities = {normalized(job(self.release, job_id)["env"]["RELEASE_TAG"])
                      for job_id in ("build-tools", "package-thin", "release-assessment", "publish-release-assets", "verify-published-assets")}
        self.assertEqual(len(identities), 1)
        identity = identities.pop()
        for event, ref, alpha, expected in (
            ("push", "v1.0.0", "", "v1.0.0"),
            ("workflow_dispatch", "main", "", ""),
            ("schedule", "main", "v1.1.0-alpha.3", "v1.1.0-alpha.3"),
            ("workflow_dispatch", "release-v1.0", "", ""),
        ):
            with self.subTest(event=event, alpha=alpha):
                self.assertEqual(
                    evaluate(identity, {"needs.alpha-tag.outputs.tag": alpha, "github.event_name": event, "github.ref_name": ref}),
                    expected,
                )
        for job_id in ("smoke-thin-plugin", "verify-published-assets"):
            with self.subTest(job_id=job_id):
                found = job(self.release, job_id)
                self.assertTrue(evaluate(found["if"], GREEN))
                self.assertFalse(evaluate(found["if"], {**GREEN, "needs.publish-release-assets.result": "skipped"}))

    def test_a_green_night_builds_and_publishes_the_alpha_in_the_same_run(self) -> None:
        alpha = job(self.release, "alpha-tag")
        self.assertEqual(needs(alpha), ["unica-ci", "rust-os"])
        night = {"inputs.alpha": True, "needs.unica-ci.result": "success", "needs.rust-os.result": "success"}
        self.assertTrue(evaluate(alpha["if"], night))
        for change in ({"inputs.alpha": False}, {"needs.unica-ci.result": "failure"}, {"needs.rust-os.result": "failure"}):
            with self.subTest(change=change):
                self.assertFalse(evaluate(alpha["if"], {**night, **change}))
        # Тег ставит и пушит только эта джоба, и только ей нужна запись в тег.
        self.assertEqual(alpha["permissions"], {"contents": "write"})
        step_by_id(alpha, "publish")
        self.assertEqual(set(alpha["outputs"]), {"tag", "sha"})
        # Ключ тегов живёт в окружении `alpha` и больше нигде не читается.
        self.assertEqual(alpha["environment"], "alpha")
        holders = [job_id for job_id, found in jobs(self.release).items()
                   if any("secrets.RELEASE_TAG_KEY" in value for value in strings(found))]
        self.assertEqual(holders, ["alpha-tag"])
        # Вся сборка идёт на служебном коммите: номер в коде равен тегу.
        sha = "${{ needs.alpha-tag.outputs.sha || github.sha }}"
        for job_id in ("build-tools", "package-thin", "p0-release-proof", "probe-thin-bootstrap", "release-assessment",
                       "publish-release-assets", "smoke-thin-plugin", "verify-published-assets"):
            with self.subTest(job_id=job_id):
                found = job(self.release, job_id)
                self.assertIn("alpha-tag", needs(found))
                self.assertEqual(steps_using(found, "actions/checkout")[0]["with"]["ref"], sha)
        self.assertIn("needs.alpha-tag.outputs.sha", script(job(self.release, "package-thin")))
        # Второго прогона нет: от alpha не зависит ни один вызов workflow.
        self.assertNotIn("gh workflow run", all_scripts(self.release))
        self.assertNotIn("gh workflow run", (REPO_ROOT / "scripts/ci/release-alpha.py").read_text(encoding="utf-8"))
        # Маркетплейс alpha не получает: он ждёт push тега и канала.
        publication = job(self.release, "gate")
        tag_push = {"needs.delivery-ci.result": "success", "github.repository": "IngvarConsulting/unica",
                    "github.event_name": "push", "github.ref_type": "tag"}
        self.assertTrue(evaluate(publication["if"], tag_push))
        self.assertFalse(evaluate(publication["if"], {**tag_push, "github.event_name": "workflow_call"}))

    def test_an_alpha_publishes_the_full_set_and_cleans_up_after_itself(self) -> None:
        publish = job(self.release, "publish-release-assets")
        release = steps_using(publish, "softprops/action-gh-release")[0]
        # Полный набор приходит из сборки и уходит в выпуск.
        full = next(found for found in downloads(publish) if found.get("pattern") == "unica-full-*")
        self.assertIn(f"{full['path']}/unica-full-*.tar.gz", release["with"]["files"])
        order = steps(publish)
        cleanup = step_by_id(publish, "prune")
        self.assertIs(cleanup["continue-on-error"], True)
        self.assertGreater(order.index(cleanup), order.index(release))
        for tag in ("v1.0.0", "v1.0.0-rc.1", "v1.0.0-alpha.2"):
            with self.subTest(tag=tag):
                self.assertEqual(evaluate(cleanup["if"], {"env.RELEASE_TAG": tag}), "-alpha." in tag)
                package = step_by_id(job(self.release, "build-tools"), "alpha-full-set")
                self.assertEqual(evaluate(package["if"], {"env.RELEASE_TAG": tag}), "-alpha." in tag)
        self.assertIn("unica-full-${{ matrix.target }}", [found.get("name") for found in uploads(job(self.release, "build-tools"))])

    def test_a_prerelease_uploads_into_a_draft_and_publishes_after_it(self) -> None:
        # Неизменяемый релиз после публикации файлов не принимает.
        publish = job(self.release, "publish-release-assets")
        release = steps_using(publish, "softprops/action-gh-release")[0]
        finish = step_by_id(publish, "publish-draft")
        for tag in ("v1.0.0", "v1.0.0-rc.1", "v1.0.0-alpha.2"):
            with self.subTest(tag=tag):
                self.assertEqual(evaluate(release["with"]["draft"], {"env.RELEASE_TAG": tag}), "-" in tag)
                self.assertEqual(evaluate(finish["if"], {"env.RELEASE_TAG": tag}), "-" in tag)
        self.assertEqual(release["with"]["tag_name"], "${{ env.RELEASE_TAG }}")
        self.assertGreater(steps(publish).index(finish), steps(publish).index(release))

    def test_conditional_pipeline_breaks_transitive_skip_propagation(self) -> None:
        dependencies = {
            "package-thin": ("needs.build-tools.result",),
            "probe-thin-bootstrap": ("needs.package-thin.result",),
            "release-assessment": ("needs.build-tools.result",),
            "publish-release-assets": ("needs.build-tools.result",),
            "smoke-thin-plugin": ("needs.package-thin.result", "needs.publish-release-assets.result"),
            "verify-published-assets": ("needs.package-thin.result", "needs.publish-release-assets.result"),
        }
        # Пропущенный alpha-tag не тянет за собой пропуск: статусная функция есть.
        scenario = {**GREEN, "github.event_name": "push"}
        probe_scenario = {**GREEN}
        for job_id, dependency_results in dependencies.items():
            with self.subTest(job_id=job_id):
                found = job(self.release, job_id)
                self.assertIn("always()", condition(found))
                values = probe_scenario if job_id == "probe-thin-bootstrap" else scenario
                self.assertTrue(evaluate(found["if"], values))
                for dependency_result in dependency_results:
                    self.assertFalse(evaluate(found["if"], {**values, dependency_result: "failure"}), dependency_result)

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
            "unica-ci": {"main": 120, "large": 300},
            "rust-os": {"main": 60, "large": 180},
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
            value = job(self.release, job_id).get("timeout-minutes")
            for profile in ("main", "large"):
                with self.subTest(job_id=job_id, profile=profile):
                    found = evaluate(value, {"inputs.profile": profile}) if isinstance(value, str) else value
                    self.assertEqual(found, minutes[profile] if isinstance(minutes, dict) else minutes)

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

        # Матрицу наборов считает шов по воротам; шаги идут по её строкам.
        for step in (python, plan):
            with self.subTest(step=step["id"]):
                assert_env_consumed(self, step, "PYTHON_MATRIX", "${{ steps.python-matrix.outputs.python_matrix }}")
                assert_env_consumed(self, step, "DOCS_ONLY", "${{ steps.classify.outputs.docs_only }}")
        self.assertEqual(gate["outputs"]["python_matrix"], "${{ steps.python-matrix.outputs.python_matrix }}")
        assert_env_consumed(self, step_by_id(gate, "python-matrix"), "DOCS_ONLY", "${{ steps.classify.outputs.docs_only }}")
        ids = [step.get("id") for step in steps(gate)]
        self.assertLess(ids.index("classify"), ids.index("python-matrix"))
        # План выгружается до тестов, как у Rust.
        self.assertLess(ids.index("python-plan"), ids.index("python"))
        # Python не идёт параллельно с Rust: MCP-тесты под нагрузкой падают
        # по таймауту. Он идёт после тестов и Clippy и с проверенным продуктом.
        for step_id in ("product", "rust-run", "clippy"):
            self.assertLess(ids.index(step_id), ids.index("python"))

    def test_agent_evaluation_and_credential_are_absent_from_every_workflow(self) -> None:
        for workflow in sorted((*WORKFLOWS_DIR.glob("*.yml"), *WORKFLOWS_DIR.glob("*.yaml"))):
            values = list(strings(load(workflow)))
            with self.subTest(workflow=workflow.name):
                self.assertFalse(any("tests/agent_evaluation" in value for value in values))
                self.assertFalse(any("CODEX_API_KEY" in value for value in values))

    def test_gate_profile_follows_the_event_not_the_job(self) -> None:
        """Кнопка и ночь передают свой профиль, тег — `release`."""
        for profile, expected in (("main", "main"), ("large", "large"), ("", "release")):
            with self.subTest(profile=profile):
                self.assertEqual(evaluate(self.release["env"]["GATE_PROFILE"], {"inputs.profile": profile}), expected)
        # Очередь слияния: ворота отвечают на merge_group, иначе очередь ждёт вечно.
        ci = load(CI_WORKFLOW)
        self.assertEqual(triggers(ci)["merge_group"], {"types": ["checks_requested"]})
        for event, expected in (("merge_group", "queue"), ("pull_request", "pr")):
            with self.subTest(event=event):
                self.assertEqual(evaluate(ci["env"]["GATE_PROFILE"], {"github.event_name": event}), expected)
        self.assertNotIn("run-tests.py --profile all", all_scripts(self.release))

    def test_line_rides_in_the_signature_and_tags_resolve_to_a_release_line(self) -> None:
        for job_id in ("unica-ci", "rust-os"):
            found = job(self.release, job_id)
            with self.subTest(job=job_id):
                line = step_by_id(found, "line")
                for name in ("REF_TYPE", "REF_NAME"):
                    self.assertRegex(line["run"], rf"\${{?{name}\b")
                for step_id in ("rust-build", "rust-run"):
                    assert_env_consumed(self, step_by_id(found, step_id), "RUN_LINE", "${{ steps.line.outputs.line }}")
        gate = job(self.release, "unica-ci")
        self.assertEqual(gate["outputs"]["line"], "${{ steps.line.outputs.line }}")
        for step_id in ("python", "python-plan"):
            assert_env_consumed(self, step_by_id(gate, step_id), "RUN_LINE", "${{ steps.line.outputs.line }}")
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
        self.assertIn("run", decide["outputs"])
        self.assertEqual(self.nightly["permissions"], {"contents": "read"})
        self.assertEqual(self.nightly["concurrency"]["group"], "unica-nightly")
        self.assertEqual(full["uses"], "$/.github/workflows/release.yml")
        self.assertEqual(needs(full), ["decide"])
        for run in ("true", "false"):
            with self.subTest(run=run):
                self.assertEqual(evaluate(full["if"], {"needs.decide.outputs.run": run}), run == "true")
        self.assertEqual(full["with"]["profile"], "large")
        # Alpha — только срез main: кнопка на другой ветке его не выпускает.
        for ref in ("refs/heads/main", "refs/heads/release-v1.0"):
            with self.subTest(ref=ref):
                self.assertEqual(evaluate(full["with"]["alpha"], {"github.ref": ref}), ref == "refs/heads/main")
        # Права вызова — верхняя граница: их хватает всем заданиям `release.yml`.
        # Вызов передаёт по имени только ключ тегов alpha: без этого секрет окружения
        # в вызванном workflow пуст. Токен маркетплейса вызову не передаётся, а
        # `id-token` нужен только Pages.
        self.assertEqual(full["secrets"], {"RELEASE_TAG_KEY": "${{ secrets.RELEASE_TAG_KEY }}"})
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
        self.assertEqual(matrix_values(job(self.release, "rust-os"), "large")["runner"], ["macos-15", "windows-latest"])
        self.assertEqual(load_gate_module().PROFILE_RUNNERS["large"], ["ubuntu-latest", "macos-15", "windows-latest"])
        self.assertEqual(
            {step["with"]["category"] for step in steps_using(gate, "github/codeql-action/upload-sarif")},
            {"zizmor", "clippy-ubuntu-latest"},
        )
        for step_id in ("removed-tests", "verify"):
            step_by_id(gate, step_id)
        for job_id in ("build-tools", "package-thin", "delivery-ci"):
            self.assertIn(job_id, jobs(self.release))

    def test_gate_waits_for_exactly_the_os_jobs_of_its_profile(self) -> None:
        """`Unica CI` и `rust-os` идут параллельно; ворота ждут джобы ОС по jobs API.

        Список ожидания и матрица ОС считаются из одного профиля: лишняя ОС —
        вечное ожидание, недостающая — красная ОС мимо ворот.
        """
        gate, other = job(self.release, "unica-ci"), job(self.release, "rust-os")
        self.assertEqual(needs(gate), [])
        self.assertEqual(needs(other), [])
        runners_step, wait = step_by_id(gate, "runners"), step_by_id(gate, "os-jobs")
        assert_env_consumed(self, wait, "RUNNERS", "${{ steps.runners.outputs.runners }}")
        for profile in ("main", "large"):
            with self.subTest(profile=profile), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "out").touch()
                result = bash(runners_step["run"], cwd=root, env={"PROFILE": profile, "GITHUB_OUTPUT": "out"})
                self.assertEqual(result.returncode, 0, result.stderr)
                runners = json.loads((root / "out").read_text(encoding="utf-8").split("runners=", 1)[1])
                matrix = matrix_values(other, profile)["runner"]
                self.assertEqual([runner for runner in runners if runner != lanes.LINUX], matrix)
                # Ворота ищут джобу ОС по имени, которое она носит.
                self.assertEqual(
                    {other["name"].replace("${{ matrix.runner }}", runner) for runner in matrix},
                    {lanes.os_job(runner) for runner in matrix},
                )
                # Ожидание покрывает срок джобы ОС с запасом на очередь раннера,
                # но кончается раньше срока самих ворот: дальше — `timed_out`.
                wait_seconds = evaluate(wait["env"]["WAIT_SECONDS"], {"inputs.profile": profile})
                os_seconds = evaluate(other["timeout-minutes"], {"inputs.profile": profile}) * 60
                self.assertGreaterEqual(wait_seconds, os_seconds)
                self.assertLessEqual(wait_seconds, os_seconds + 1200)
                self.assertLess(wait_seconds, evaluate(gate["timeout-minutes"], {"inputs.profile": profile}) * 60)
        self.assertRegex(wait["run"], r"\$\{?WAIT_SECONDS\b")
        # Результаты ОС читаются только после того, как джобы ОС закончились.
        order = steps(gate)
        fetch = next(step for step in steps_using(gate, "actions/download-artifact")
                     if "steps.os-jobs.outcome == 'success'" in condition(step))
        self.assertLess(order.index(wait), order.index(fetch))
        self.assertLess(order.index(fetch), order.index(step_by_id(gate, "verify")))
        self.assertTrue(evaluate(fetch["if"], {"steps.os-jobs.outcome": "success"}))
        self.assertFalse(evaluate(fetch["if"], {"steps.os-jobs.outcome": "failure"}))

    def test_every_downloaded_artifact_is_uploaded_by_a_job_it_waits_for(self) -> None:
        """Каждое скачивание находит выгрузку того же прогона, которую ждёт.

        Источник — задание из предков по `needs`; результаты ОС `Unica CI`
        ждёт через jobs API (см. test_gate_waits_for_exactly_the_os_jobs_of_its_profile).
        Переименованный артефакт иначе краснел бы только в настоящем прогоне.
        """
        produced: dict[str, set[str]] = {}
        for job_id, found in jobs(self.release).items():
            for upload in uploads(found):
                for name in artifact_names(found, upload.get("name", "artifact")):
                    produced.setdefault(name, set()).add(job_id)
        checked = 0
        for job_id, found in jobs(self.release).items():
            for step in steps_using(found, "actions/download-artifact"):
                requested = step["with"].get("name") or step["with"].get("pattern")
                upstream = ancestors(self.release, job_id) | {job_id}
                if "steps.os-jobs.outcome == 'success'" in condition(step):
                    upstream.add("rust-os")
                for pattern in (expanded for name in artifact_names(found, requested) for expanded in expand_braces(name)):
                    checked += 1
                    with self.subTest(job=job_id, pattern=pattern):
                        matched = {name for name in produced if fnmatch.fnmatchcase(name, pattern)}
                        self.assertTrue(matched, f"{pattern} никто не выгружает")
                        for name in matched:
                            self.assertTrue(produced[name] & upstream, f"{name} выгружает {produced[name]}, а {job_id} их не ждёт")
        self.assertGreater(checked, 10)

    def test_pages_are_jobs_of_the_run_that_reports_them(self) -> None:
        """Сайт обновляется заданием прогона: и зелёного, и красного, но не чужой ветки."""
        build, deploy = job(self.release, "pages-build"), job(self.release, "pages-deploy")

        # После итога сборки и маркетплейса, даже красных; `alpha-tag` называет
        # версию, чьи тесты дал прогон.
        self.assertEqual(set(needs(build)), {"delivery-ci", "promote", "alpha-tag"})
        own = {"github.repository": "IngvarConsulting/unica", "github.ref": "refs/heads/main",
               "github.ref_name": "main", "github.ref_type": "branch"}
        for case, expected in (
            ({}, True),
            ({"github.ref": "refs/heads/release-v1.0", "github.ref_name": "release-v1.0"}, True),
            ({"github.ref": "refs/tags/v1.0.0", "github.ref_name": "v1.0.0", "github.ref_type": "tag"}, True),
            ({"github.ref": "refs/heads/feature", "github.ref_name": "feature"}, False),
            ({"github.repository": "fork/unica"}, False),
        ):
            with self.subTest(case=case):
                self.assertEqual(evaluate(build["if"], {**own, **case}), expected)
        self.assertNotIn("result == 'success'", condition(build))
        # Результаты — свои, артефакты чужого прогона больше не читаются.
        download = steps_using(build, "actions/download-artifact")[0]
        self.assertNotIn("run-id", download["with"])
        self.assertNotIn("workflow_run", "\n".join(strings(build)))
        # Страницы всегда из main: сайт двигается только вперёд.
        checkouts = steps_using(build, "actions/checkout")
        self.assertEqual(len(checkouts), 1)
        self.assertEqual(checkouts[0]["with"]["ref"], "main")
        self.assertEqual(build["permissions"], {"contents": "read", "actions": "read"})
        # Право писать на Pages есть только у публикации, и тег alpha её не проходит.
        self.assertEqual(needs(deploy), ["pages-build"])
        self.assertEqual(deploy["permissions"], {"pages": "write", "id-token": "write"})
        self.assertEqual(deploy["environment"]["name"], "github-pages")
        for ref_name, result, expected in (("main", "success", True), ("v1.0.0-alpha.1", "success", False),
                                           ("main", "failure", False)):
            with self.subTest(ref_name=ref_name, result=result):
                self.assertEqual(evaluate(deploy["if"], {"github.ref_name": ref_name, "needs.pages-build.result": result}), expected)
        writers = [job_id for job_id, found in jobs(self.release).items() if (found.get("permissions") or {}).get("pages")]
        self.assertEqual(writers, ["pages-deploy"])
        for found in (build, deploy):
            self.assertEqual(found["concurrency"], {"group": "unica-pages", "cancel-in-progress": False})

    def test_pages_deploy_retries_once_and_fails_on_the_second_refusal(self) -> None:
        """Pages отвечает отказом на отдельные развёртывания: один повтор, потом красный."""
        deploy = job(self.release, "pages-deploy")
        first, second = step_by_id(deploy, "deployment"), step_by_id(deploy, "redeployment")
        self.assertEqual(action(first), "actions/deploy-pages")
        self.assertEqual(first["uses"], second["uses"])
        self.assertIs(first["continue-on-error"], True)
        self.assertNotIn("continue-on-error", second)
        order = steps(deploy)
        self.assertLess(order.index(first), order.index(second))
        between = order[order.index(first) + 1:order.index(second)]
        for outcome in ("success", "failure"):
            with self.subTest(outcome=outcome):
                values = {"steps.deployment.outcome": outcome}
                self.assertEqual(evaluate(second["if"], values), outcome == "failure")
                for step in between:
                    self.assertEqual(evaluate(step["if"], values), outcome == "failure")
        # Адрес окружения берётся у того развёртывания, которое прошло.
        url = deploy["environment"]["url"]
        self.assertEqual(evaluate(url, {"steps.deployment.outputs.page_url": "a", "steps.redeployment.outputs.page_url": ""}), "a")
        self.assertEqual(evaluate(url, {"steps.deployment.outputs.page_url": "", "steps.redeployment.outputs.page_url": "b"}), "b")

    def test_publication_queue_keeps_every_admitted_release(self) -> None:
        """Прогоны тегов ждут в одной очереди, проверки веток в неё не входят.

        С одним местом ожидания третий выпуск снимал бы второй. Порядок внутри
        очереди GitHub не обещает: его держит страж порядка версий в stage и promote.
        """
        concurrency = self.release["concurrency"]
        groups = {}
        for ref_type, ref, profile in (("tag", "refs/tags/v1.0.0", ""), ("tag", "refs/tags/v1.1.0", ""),
                                       ("branch", "refs/heads/main", "main"), ("branch", "refs/heads/main", "large"),
                                       ("branch", "refs/heads/release-v1.0", "main")):
            groups[(ref, profile)] = evaluate(concurrency["group"], {"github.ref_type": ref_type, "github.ref": ref, "inputs.profile": profile})
        self.assertEqual(groups[("refs/tags/v1.0.0", "")], groups[("refs/tags/v1.1.0", "")])
        self.assertEqual(len(set(groups.values())), 4)
        self.assertEqual(concurrency["queue"], "max")
        self.assertIs(concurrency["cancel-in-progress"], False)

    def test_guards_ship_findings_to_code_scanning_not_the_gate(self) -> None:
        """Находка линтера — не исход теста: SARIF в Code Scanning, гейт не краснеет."""
        gate = job(self.release, "unica-ci")
        sarif = next(step for step in steps_using(gate, "github/codeql-action/upload-sarif") if step["with"]["category"] == "zizmor")
        zizmor = step_by_id(gate, "guards-zizmor")

        # Отчёт, который выгружается, пишет сам шаг zizmor, и находки не валят шаг.
        self.assertIn(sarif["with"]["sarif_file"], zizmor["run"])
        self.assertIn("--no-exit-codes", zizmor["run"])
        self.assertLess(steps(gate).index(zizmor), steps(gate).index(sarif))
        assert_pinned(self, RELEASE_WORKFLOW, sarif, "v4")
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
        lock = step_by_id(build, "lock")

        self.assertIs(cache.get("continue-on-error"), True)
        assert_pinned(self, RELEASE_WORKFLOW, cache, "v6")
        self.assertEqual(cache["with"]["path"], ".build/tool-work/${{ matrix.target }}/cargo-target")
        key = normalized(cache["with"]["key"])
        for part in ("runner.os", "matrix.target", "steps.rust-toolchain.outputs.cachekey", "steps.lock.outputs.hash"):
            self.assertIn(part, key)
        self.assertNotIn("restore-keys", cache["with"])
        order = steps(build)
        self.assertLess(order.index(lock), order.index(cache))
        self.assertLess(order.index(cache), order.index(step_by_id(build, "build")))

        # Номер alpha в Cargo.lock не меняет ключ: иначе каждая ночь писала бы новую запись.
        def lock_hash(text: str) -> str:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "Cargo.lock").write_text(text, encoding="utf-8", newline="\n")
                (root / "out").touch()
                result = bash(lock["run"], cwd=root, env={"GITHUB_OUTPUT": "out"})
                self.assertEqual(result.returncode, 0, result.stderr)
                return (root / "out").read_text(encoding="utf-8").strip()

        package = '[[package]]\nname = "unica"\nversion = "0.15.0{}"\n\n[[package]]\nname = "serde"\nversion = "{}"\n'
        base = lock_hash(package.format("-alpha.1", "1.0.0"))
        self.assertEqual(base, lock_hash(package.format("-alpha.2", "1.0.0")))
        self.assertNotEqual(base, lock_hash(package.format("-alpha.1", "1.0.1")))

        # Отчёт идёт всегда и называет исход кэша.
        report = step_by_id(build, "build-metrics")
        self.assertEqual(condition(report), "always()")
        for outcome, hit, expected in (("success", "true", "exact-hit"), ("success", "false", "miss"), ("failure", "", "error")):
            with self.subTest(outcome=outcome, hit=hit), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "metrics.json").write_text(json.dumps({"cargoBuildSeconds": 42}), encoding="utf-8")
                (root / "summary").touch()
                env = {"CACHE_STEP_OUTCOME": outcome, "CACHE_HIT": hit, "METRICS_FILE": "metrics.json",
                       "TARGET": "linux-x64", "GITHUB_STEP_SUMMARY": "summary"}
                result = bash(report["run"], cwd=root, env=env, prelude=python_prelude())
                self.assertEqual(result.returncode, 0, result.stderr)
                summary = (root / "summary").read_text(encoding="utf-8")
                self.assertIn(expected, summary)
                self.assertIn("42", summary)
        self.assertEqual(normalized(report["env"]["CACHE_STEP_OUTCOME"]), "${{ steps.cargo-cache.outcome }}")
        self.assertEqual(normalized(report["env"]["CACHE_HIT"]), "${{ steps.cargo-cache.outputs.cache-hit }}")

    def test_runtime_matrix_builds_verifies_and_exports_narrow_artifacts(self) -> None:
        build = job(self.release, "build-tools")
        names = [upload.get("name") for upload in uploads(build)]
        paths = [upload.get("path") for upload in uploads(build)]

        self.assertEqual({target for target, _ in targets(build)}, {"darwin-arm64", "linux-x64", "win-x64"})
        self.assertNotIn("package-runtime", jobs(self.release))
        self.assertFalse(any("unica-tools-" in value for value in strings(self.release)))
        for step_id in ("build", "package-runtime", "verify-runtime"):
            step_by_id(build, step_id)
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
        self.assertGreaterEqual(sum(1 for upload in uploads(build) if upload.get("retention-days") == 1), 3)

    def test_manual_macos_acceptance_exports_core_bytes_without_publishing_a_release(self) -> None:
        build = job(self.release, "build-tools")
        upload = upload_step(build, "unica-runtime-${{ matrix.target }}")

        for target in ("linux-x64", "darwin-arm64", "win-x64"):
            for event, tag in (("workflow_dispatch", ""), ("schedule", ""), ("push", "v0.15.0"),
                               ("schedule", "v0.15.0-alpha.1"), ("workflow_dispatch", "v0.15.0-alpha.1")):
                values = {**GREEN, "matrix.target": target, "github.event_name": event, "env.RELEASE_TAG": tag,
                          "needs.alpha-tag.outputs.tag": tag if event != "push" else "",
                          "needs.alpha-tag.result": "success" if tag and event != "push" else "skipped"}
                expected = target == "linux-x64" or (target == "darwin-arm64" and event != "push") or bool(tag)
                with self.subTest(target=target, event=event, tag=tag):
                    self.assertEqual(evaluate(upload["if"], values), expected)
                    self.assertTrue(evaluate(build["if"], values))
                    self.assertEqual(evaluate(job(self.release, "publish-release-assets")["if"], values), bool(tag))
        self.assertEqual(upload["with"]["retention-days"], 1)

    def test_mcp_smoke_runs_against_extracted_deterministic_runtime(self) -> None:
        build = job(self.release, "build-tools")
        order = [step.get("id") for step in steps(build)]
        self.assertLess(order.index("package-runtime"), order.index("extract-runtime"))
        self.assertLess(order.index("extract-runtime"), order.index("mcp-smoke"))
        self.assertEqual(step_by_id(build, "mcp-smoke").get("timeout-minutes"), 3)

    def test_thin_payload_downloads_only_metadata_and_bootstrap(self) -> None:
        thin = job(self.release, "package-thin")
        patterns = [download.get("pattern") for download in downloads(thin)]
        marketplace = next(upload for upload in uploads(thin) if upload.get("name") == "unica-thin-marketplace")

        self.assertEqual(needs(thin), ["build-tools", "alpha-tag"])
        self.assertEqual(set(patterns), {"unica-metadata-*", "unica-bootstrap-*"})
        self.assertIs(marketplace.get("include-hidden-files"), True)
        self.assertEqual(marketplace.get("retention-days"), 90)
        self.assertNotIn("unica-codex-marketplace-${{ matrix.target }}", list(strings(self.release)))

    def test_package_and_publication_file_gates_require_all_three_manifests(self) -> None:
        gates = (
            (job(self.release, "package-thin"), "host-contracts", "dist/thin/marketplace"),
            (job(self.publish, "stage"), "contracts", "payload"),
            (job(self.publish, "promote"), "catalog", "payload"),
        )
        manifests = (
            "plugins/unica/.codex-plugin/plugin.json",
            "plugins/unica/.claude-plugin/plugin.json",
            "plugins/unica/.zcode-plugin/plugin.json",
        )
        for node, step_id, prefix in gates:
            gate = step_by_id(node, step_id)
            with self.subTest(step=step_id), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                for relative in (*manifests, "plugins/unica/.mcp.json", "plugins/unica/runtime-manifest.json", ".agents/plugins/marketplace.json", ".claude-plugin/marketplace.json"):
                    target = root / prefix / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text("{}\n", encoding="utf-8")
                # Only the download is replaced; execute the workflow's file gate.
                valid = bash(gate["run"], cwd=root, prelude="gh() { :; }\n")
                self.assertEqual(valid.returncode, 0, valid.stderr)
                for relative in manifests:
                    with self.subTest(missing=relative):
                        target = root / prefix / relative
                        target.unlink()
                        missing = bash(gate["run"], cwd=root, prelude="gh() { :; }\n")
                        target.write_text("{}\n", encoding="utf-8")
                        self.assertNotEqual(missing.returncode, 0, f"{step_id} accepted missing {relative}")

    def test_intermediate_non_marketplace_artifacts_expire_after_one_day(self) -> None:
        assessment = job(self.release, "release-assessment")

        self.assertEqual([(upload["name"], upload["retention-days"]) for upload in uploads(assessment)], [("unica-release-assessment", 1)])

    def test_packaged_bootstrap_is_smoked_on_every_supported_host(self) -> None:
        windows_cleanup = step_by_id(job(self.release, "build-tools"), "bootstrap-cleanup-windows")
        for target in ("win-x64", "linux-x64"):
            with self.subTest(target=target):
                self.assertEqual(evaluate(windows_cleanup["if"], {"matrix.target": target}), target == "win-x64")
        probe = job(self.release, "probe-thin-bootstrap")
        smoke = job(self.release, "smoke-thin-plugin")
        expected_targets = {
            "linux-x64": "ubuntu-latest",
            "win-x64": "windows-2022",
            "darwin-arm64": "macos-15",
        }

        self.assertEqual(dict(targets(probe)), expected_targets)
        self.assertEqual(dict(targets(smoke)), expected_targets)
        step_by_id(probe, "probe")
        step_by_id(smoke, "smoke")
        self.assertEqual(needs(probe), ["package-thin", "alpha-tag"])
        self.assertEqual(needs(smoke), ["package-thin", "publish-release-assets", "alpha-tag"])

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
        # носят имена вне него, а не полагаются на фильтр `files:` (п. 4).
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
        # Ядро и его описание сводятся и сверяются до выкладки.
        order = steps(publish)
        self.assertLess(order.index(downloaded[-1]), order.index(step_by_id(publish, "pair")))
        self.assertLess(order.index(step_by_id(publish, "pair")), order.index(release))
        self.assertFalse(any("install-unica" in value for value in strings(publish)))
        # Выложенное перекачивается из выпуска и сверяется заново.
        self.assertLess(steps(verify).index(step_by_id(verify, "download-release")), len(steps(verify)) - 1)

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
        self.assertNotIn("publish-release-assets", ancestors(self.release, "release-assessment"))
        self.assertFalse(any("publish-release-assets" in value for value in strings(assessment)))
        self.assertEqual(upload["with"]["name"], "unica-release-assessment")
        self.assertEqual(condition(upload), "always()")

    def test_pr_permissions_are_read_only_and_cross_repo_write_uses_secret(self) -> None:
        self.assertEqual(self.release["permissions"], {"contents": "read"})
        self.assertEqual(self.publish["permissions"], {"contents": "read"})
        holders = {job_id for job_id, found in jobs(self.publish).items()
                   if "secrets.UNICA_MARKETPLACE_TOKEN" in "\n".join(strings(found))}
        self.assertEqual(holders, set(PUBLISH_JOBS) - {"gate"})
        # Пишут в маркетплейс stage, anchor и promote; проверки установки только читают.
        writers = {job_id for job_id in holders
                   if job(self.publish, job_id)["env"]["GH_TOKEN"] == "${{ secrets.UNICA_MARKETPLACE_TOKEN }}"}
        self.assertEqual(writers, {"stage", "anchor", "promote"})
        # Токен есть только у заданий окружения `marketplace`, и кода тега там нет.
        for job_id in holders:
            found = job(self.publish, job_id)
            with self.subTest(token_job=job_id):
                self.assertEqual(found["environment"], "marketplace")
                checkouts = steps_using(found, "actions/checkout")
                self.assertEqual(len(checkouts), 1)
                self.assertEqual(checkouts[0]["with"]["ref"], "main")
        # Окружение `marketplace` есть только у заданий с токеном.
        in_environment = {job_id for job_id, found in jobs(self.publish).items() if found.get("environment") == "marketplace"}
        self.assertEqual(in_environment, holders)
        for job_id, found in jobs(self.publish).items():
            with self.subTest(job_id=job_id):
                self.assertNotEqual((found.get("permissions") or {}).get("pull-requests"), "write")

    def test_cross_repository_push_configures_git_credentials(self) -> None:
        # Каждое задание, которое пушит в маркетплейс, настраивает Git до push.
        for job_id in ("stage", "anchor", "promote"):
            found = job(self.publish, job_id)
            with self.subTest(job=job_id):
                texts = [step.get("run", "") for step in steps(found)]
                configured = next(index for index, text in enumerate(texts) if "gh auth setup-git" in text)
                pushed = next(index for index, text in enumerate(texts) if re.search(r"\bpush\b", text) and "git" in text)
                self.assertLess(configured, pushed)

    def test_publication_is_one_linear_pass_ordered_by_needs(self) -> None:
        """Publication order: stage → anchor → verify → promote, no pull requests, no warden.

        The order is the contract: the candidate anchor exists before the
        install checks run, and the catalog moves — with the release tag that
        it names — only behind their green result. The steps themselves,
        reruns included, are executed in test_publish_channels.
        """
        on = triggers(self.publish)
        text = "\n".join(script(job(self.publish, job_id)) for job_id in PUBLISH_JOBS)
        gate = job(self.publish, "gate")

        # Публикацию открывает только зелёная сборка этого же прогона по push тега.
        self.assertEqual(needs(gate), ["delivery-ci"])
        tag_push = {"needs.delivery-ci.result": "success", "github.repository": "IngvarConsulting/unica",
                    "github.event_name": "push", "github.ref_type": "tag"}
        self.assertTrue(evaluate(gate["if"], tag_push))
        for change in ({"needs.delivery-ci.result": "failure"}, {"github.repository": "fork/unica"},
                       {"github.event_name": "workflow_dispatch"}, {"github.ref_type": "branch"}):
            with self.subTest(change=change):
                self.assertFalse(evaluate(gate["if"], {**tag_push, **change}))
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
        # An existing tag is proven identical, never moved; nothing forces a ref.
        self.assertNotIn("git tag -f", text)
        self.assertNotIn("--force", text)
        self.assertNotIn("sort -V", text)
        # The payload is trusted only from this run's own tag.
        own = step_by_id(job(self.publish, "stage"), "own-tag")
        self.assertEqual(normalized(own["env"]["RELEASE_TAG"]), "${{ steps.identity.outputs.release_tag }}")
        with tempfile.TemporaryDirectory() as tmp:
            for ref_name, accepted in (("v1.0.0", True), ("v1.0.1", False)):
                with self.subTest(ref_name=ref_name):
                    result = bash(own["run"], cwd=Path(tmp), env={"GITHUB_REF_NAME": ref_name, "RELEASE_TAG": "v1.0.0"},
                                  prelude="gh() { :; }\n")
                    self.assertEqual(result.returncode == 0, accepted, result.stderr)


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
        self.assertIn("verify-delivery-reachable.py", script(job(self.release, "verify-published-assets")))
        step_by_id(job(self.release, "smoke-thin-plugin"), "prefetch")

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
        engine = upload_step(self.build, "unica-assessment-engine-linux-x64")

        self.assertEqual(evaluate(engine["if"], {"matrix.target": "linux-x64"}), True)
        self.assertIn("stage-unica-assessment-engine.py", script(self.build))
        self.assertIn("unica-assessment-engine-linux-x64", [download.get("name") for download in downloads(assessment)])

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

        order = steps(self.build)
        lifecycle = (
            step_by_id(self.build, "build"),
            step_by_id(self.build, "package-runtime"),
            step_by_id(self.build, "verify-runtime"),
            upload_step(self.build, "unica-metadata-${{ matrix.target }}"),
            upload_step(self.build, "unica-bootstrap-${{ matrix.target }}"),
            upload_step(self.build, "unica-runtime-${{ matrix.target }}"),
        )
        positions = [order.index(step) for step in lifecycle]
        self.assertEqual(positions, sorted(positions))

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
                scenario = {**GREEN, "github.event_name": "push"}
                self.assertFalse(evaluate(found["if"], {**scenario, f"needs.{dependency}.result": "failure"}), job_id)

        # Перекачанные ассеты сверяются для всех целей сразу.
        published = job(self.release, "verify-published-assets")
        self.assertNotIn("strategy", published)
        self.assertNotIn("--target", script(published), "published verification must cover every target")

        smoke = job(self.release, "smoke-thin-plugin")
        order = steps(smoke)
        self.assertLess(order.index(step_by_id(smoke, "smoke")), order.index(step_by_id(smoke, "prefetch")))
        prefetch = step_by_id(smoke, "prefetch")
        for target in ("linux-x64", "win-x64"):
            with self.subTest(target=target):
                self.assertEqual(evaluate(prefetch["if"], {"matrix.target": target}), target == "linux-x64")


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

        for tag in ("v1.0.0", "v1.0.0-rc.1"):
            with self.subTest(tag=tag):
                self.assertEqual(evaluate(release["with"]["prerelease"], {"env.RELEASE_TAG": tag}), "-" in tag)

    def test_publication_asks_first_which_channel_the_release_reaches(self) -> None:
        gate = job(self.publish, "gate")
        decide = step_by_id(gate, "decide")

        self.assertEqual(gate["outputs"], {"channel": "${{ steps.decide.outputs.channel }}"})
        # Шаг исполняется с правилами канала из main и называет канал по тегу.
        for tag, channel in (("v1.0.0", "stable"), ("v1.0.0-rc.2", "next")):
            with self.subTest(tag=tag), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                rules = root / "rules/scripts/ci"
                rules.mkdir(parents=True)
                shutil.copy(REPO_ROOT / "scripts/ci/release-channel.py", rules / "release-channel.py")
                for name in ("out", "summary"):
                    (root / name).touch()
                env = {"GITHUB_REF_NAME": tag, "GITHUB_OUTPUT": "out", "GITHUB_STEP_SUMMARY": "summary"}
                result = bash(decide["run"], cwd=root, env=env, prelude=python_prelude())
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual((root / "out").read_text(encoding="utf-8").split(), [f"channel={channel}"])

    def test_every_publishing_stage_waits_for_that_answer(self) -> None:
        # Достаточно загейтить первую стадию: остальные ждут её через `needs`.
        # Предвыпуск не `-rc.N` существует для замеров и не публикуется.
        stage = job(self.publish, "stage")

        self.assertEqual(needs(stage), ["gate"])
        for channel, result, expected in (("stable", "success", True), ("next", "success", True),
                                          ("none", "success", False), ("", "success", False),
                                          ("stable", "failure", False)):
            with self.subTest(channel=channel, result=result):
                values = {"needs.gate.result": result, "needs.gate.outputs.channel": channel}
                self.assertEqual(evaluate(stage["if"], values), expected)

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
            if ancestors(self.release, job_id) & skippable:
                with self.subTest(job_id=job_id):
                    gate = condition(found)
                    self.assertTrue("!cancelled()" in gate or "always()" in gate, gate)

    def test_channel_rules_come_from_main_without_credentials(self) -> None:
        # Код тега не исполняется рядом с токеном маркетплейса: правила канала
        # берутся из main, и checkout не оставляет токена.
        for job_id in PUBLISH_JOBS:
            with self.subTest(job_id=job_id):
                checkouts = steps_using(job(self.publish, job_id), "actions/checkout")
                self.assertEqual(len(checkouts), 1)
                self.assertEqual(
                    checkouts[0]["with"],
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
