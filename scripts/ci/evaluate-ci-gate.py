#!/usr/bin/env python3
"""Evaluate the stable aggregate result for the routed Unica CI workflow."""

from __future__ import annotations

import json
import os
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import NamedTuple

sys.path.insert(0, str(Path(__file__).resolve().parent))
from workflow_routing import CACHE_JOB, RUST_JOBS, python_matrix as routed_python_matrix, rust_route  # noqa: E402

LINUX = "ubuntu-latest"
# Gate profile → runners. Mirrors `ci_rust_lanes.profile_hosts`; imported
# lazily there would pull Cargo helpers into the gate, so the table is small
# and its equality is a test.
PROFILE_RUNNERS = {
    "pr": [LINUX, "macos-14"], "queue": [LINUX, "macos-14"], "main": [LINUX, "macos-14"],
    "release": [LINUX, "macos-14"], "large": [LINUX, "macos-14", "windows-latest"],
}


CLASSIFICATION_OUTPUTS = (
    "rust_changed",
    "platform_changed",
    "toolchain_changed",
    "search_integration_changed",
    "package_changed",
    "plugin_content_changed",
    "ci_changed",
    "release_required",
    "assessment_required",
)
SCOPE_OUTPUTS = (*CLASSIFICATION_OUTPUTS, "docs_only")
ALWAYS_JOBS = (
    "classify-changes",
    "guards",
    "test-python",
)
PYTHON_PRODUCT_JOB = "build-python-product"
PACKAGE_JOBS = (
    "build-tools",
    "package-thin",
)
ASSESSMENT_JOB = "release-assessment"
P0_PROOF_JOB = "p0-release-proof"
PUBLISH_JOBS = (
    "publish-release-assets",
    "smoke-thin-plugin",
    "verify-published-assets",
)
# Pull request medium of affected packages and the jobs of the other runners
# are separate entries: a skip of one must not hide inside another's success.
RUST_MEDIUM = "rust-medium"
OS_JOBS = "os-jobs"
# What the `Unica CI` job proves itself; the delivery tail of a tag or a manual
# run is judged by `Delivery CI` after it, over the same table.
DELIVERY_ONLY = (*PACKAGE_JOBS, ASSESSMENT_JOB, P0_PROOF_JOB, "probe-thin-bootstrap", *PUBLISH_JOBS)

# Steps of the `Unica CI` job behind each gate entry. A group is green only
# when every step of it succeeded and skipped only when every step skipped:
# a partly skipped group is a wiring defect, not a pass.
STEP_GROUPS = {
    "classify-changes": ("classify",),
    "guards": ("guards-compile", "guards-version", "guards-fmt", "guards-zizmor"),
    "build-python-product": ("product",),
    "test-python": ("python-plan", "python", "pr-python"),
    "test-rust-platforms": ("shadow", "rust-build", "rust-run", "verify"),
    # Добавки pull request к Rust: medium затронутых пакетов и проверка
    # объявления снятых тестов (#1938); обе идут только в PR с Rust.
    RUST_MEDIUM: ("rust-medium", "removed-tests"),
    # Clippy — шаг `clippy` (в фоне, очередь и ветки) или ожидание шага
    # джобы `Lint and Python` (pull request).
    "lint-rust-platforms": ("clippy", "lint-job"),
    CACHE_JOB: ("warm",),
}


def combined(outcomes: list[str]) -> str:
    if not outcomes or any(outcome == "missing" for outcome in outcomes):
        return "missing"
    for verdict in ("failure", "cancelled"):
        if verdict in outcomes:
            return verdict
    # A named non-success (`timed_out`, `not_acquired`) reaches the summary as is.
    named = [outcome for outcome in outcomes if outcome not in ("success", "skipped")]
    if named:
        return named[0]
    if all(outcome == "success" for outcome in outcomes):
        return "success"
    if all(outcome == "skipped" for outcome in outcomes):
        return "skipped"
    return "partial"


# Entries whose steps run inside `Unica CI` on most events and in the second
# pull request job (`Lint and Python`) on a pull request: exactly one of the
# alternatives runs. The skipped one does not make the entry partial; both
# running, or a part of one, does.
ALTERNATIVE_GROUPS = {
    "lint-rust-platforms": (("clippy",), ("lint-job",)),
    "test-python": (("python-plan", "python"), ("pr-python",)),
}


def validation_results(steps: Mapping, os_jobs: Mapping[str, str] | None, profile: str | None) -> dict[str, str]:
    """Gate entries from the step outcomes of `Unica CI` and the other runners' jobs."""
    results = {}
    for entry, ids in STEP_GROUPS.items():
        outcomes = []
        for step in ids:
            details = steps.get(step)
            outcomes.append(details.get("outcome", "missing") if isinstance(details, Mapping) else "missing")
        if entry in ALTERNATIVE_GROUPS:
            by_step = dict(zip(ids, outcomes))
            parts = [[by_step[step] for step in part] for part in ALTERNATIVE_GROUPS[entry]]
            active = [part for part in parts if any(outcome != "skipped" for outcome in part)]
            outcomes = active[0] if len(active) == 1 else outcomes if not active else [*sum(active, []), "partial"]
        results[entry] = combined(outcomes)
    if os_jobs is None:
        results[OS_JOBS] = "skipped"
    else:
        # Windows stays advisory in the nightly until #1523: any outcome of its
        # job is accepted there, as `continue-on-error` did, but it must exist.
        accepted = [
            "success" if outcome == "success" or (profile == "large" and name == "Rust (windows-latest)" and outcome != "missing")
            else outcome for name, outcome in sorted(os_jobs.items())
        ]
        results[OS_JOBS] = combined(accepted) if accepted else "missing"
    return results


class GateEvaluation(NamedTuple):
    event_name: str
    ref: str
    classification: dict[str, str]
    contour: str
    results: dict[str, str]
    expected: dict[str, str]
    unexpected: dict[str, tuple[str, str]]
    skipped_jobs: list[str]

    @property
    def ok(self) -> bool:
        return not self.unexpected


def _validated_classification(
    outputs: Mapping[str, str],
) -> tuple[dict[str, bool], tuple[str, str] | None]:
    invalid = [
        f"{name}={outputs.get(name, 'missing')}"
        for name in SCOPE_OUTPUTS
        if outputs.get(name) not in {"true", "false"}
    ]
    if invalid:
        return {}, (", ".join(invalid), "all typed outputs are true or false")

    values = {name: outputs[name] == "true" for name in SCOPE_OUTPUTS}
    contradictions: list[str] = []
    if values["docs_only"] and any(values[name] for name in CLASSIFICATION_OUTPUTS):
        contradictions.append("docs_only cannot enable code contours")
    if values["platform_changed"] and not (values["rust_changed"] or values["ci_changed"]):
        contradictions.append("platform_changed requires rust_changed or ci_changed")
    if values["toolchain_changed"] and not (
        values["rust_changed"] and values["package_changed"] and values["release_required"]
    ):
        contradictions.append("toolchain_changed requires rust/package/release contours")
    if values["package_changed"] and not values["release_required"]:
        contradictions.append("package_changed requires release_required")
    if values["assessment_required"] and not (
        values["release_required"] or values["ci_changed"]
    ):
        contradictions.append("assessment_required requires release or CI contours")
    if contradictions:
        return values, ("; ".join(contradictions), "a consistent classification")
    return values, None


def expected_results(
    event_name: str,
    ref: str,
    classification: Mapping[str, str],
    *,
    gate_profile: str | None = None,
    python_matrix: str | None = None,
    runners: str | None = None,
    scope: str = "delivery",
) -> tuple[str, dict[str, str], dict[str, tuple[str, str]]]:
    expected = {job: "success" for job in ALWAYS_JOBS}
    invalid: dict[str, tuple[str, str]] = {}
    values, classification_error = _validated_classification(classification)

    if classification_error is not None:
        invalid["classification"] = classification_error
        values = {name: False for name in SCOPE_OUTPUTS}

    is_tag = event_name == "push" and ref.startswith("refs/tags/")
    # Push в `main` приходит из очереди слияния: это дерево уже проверено на
    # `merge_group`. Отдельная compile-only джоба может обновить кэш;
    # она не исполняет тесты и не заменяет проверку очереди.
    is_main_push = event_name == "push" and ref == "refs/heads/main"
    # Push в релизную линию — ворота линии: очереди там нет, поэтому полный
    # набор тестов без упаковки. Отсюда сайт собирает отчёт линии.
    is_branch = event_name == "push" and ref.startswith("refs/heads/") and not is_main_push
    # Кодовая очередь — полный набор. Исключение для известной прозы задаёт
    # classifier по всему immutable диапазону группы, а не по одному PR.
    is_queue = event_name == "merge_group"
    is_manual = event_name == "workflow_dispatch"
    is_pr = event_name == "pull_request"
    if not (is_tag or is_main_push or is_branch or is_queue or is_manual or is_pr):
        invalid["event"] = (f"{event_name}:{ref}", "pull_request, merge_group, branch push, tag push, or workflow_dispatch")
    profiles = {"main", "large"} if is_manual else {
        "release" if is_tag else "queue" if is_queue else "pr" if is_pr else "main"
    }
    if gate_profile not in profiles:
        invalid["gate_profile"] = (gate_profile, ", ".join(sorted(profiles)))

    docs_only = values["docs_only"]
    if docs_only and not (is_pr or is_queue):
        invalid["docs_only"] = ("true", "documentation exception only on PR or merge_group")
    if (is_tag or is_branch or (is_queue and not docs_only) or is_manual) and not all(values[name] for name in CLASSIFICATION_OUTPUTS):
        invalid["classification"] = (
            ", ".join(name for name in CLASSIFICATION_OUTPUTS if not values[name]) or "invalid",
            "all contours enabled for tag, release-line push, merge_group or workflow_dispatch",
        )

    # A nonempty matrix alone could silently omit medium or an entire suite.
    # Compare all rows against the same selection policy as the runner.
    if python_matrix is not None:
        try:
            matrix = json.loads(python_matrix)
        except json.JSONDecodeError:
            matrix = None
        if not isinstance(matrix, list):
            invalid["python_matrix"] = (python_matrix, "a JSON list of Python suites")
        elif gate_profile in profiles:
            try:
                expected_matrix = routed_python_matrix(gate_profile, docs_only=docs_only)
            except ValueError as error:
                invalid["python_matrix"] = (python_matrix, str(error))
            else:
                if matrix != expected_matrix and not (is_main_push and matrix == []):
                    invalid["python_matrix"] = (
                        python_matrix, json.dumps(expected_matrix, separators=(",", ":")),
                    )
    else:
        invalid["python_matrix"] = ("missing", "the classified Python matrix")

    # The classifier emits this same policy for workflow conditions. The gate
    # derives it again from validated scope: missing routing outputs cannot
    # turn a required job into an accepted skip.
    route = rust_route(event_name, ref, values, docs_only=docs_only)
    if is_main_push:
        expected["test-python"] = "skipped"
    expected[PYTHON_PRODUCT_JOB] = (
        "success" if not (is_main_push or is_pr or docs_only) else "skipped"
    )
    # Сборка пакета и холодные старты сняты с pull request до пересборки системы
    # тестирования: прослеживаемости они не давали, а гейт красили. Тег и ручной
    # запуск их сохраняют — выпуск обязан собираться. Push в ветку упаковку тоже
    # не гоняет: это ворота тестов, а упаковка — дело тега.
    package_pipeline = (values["release_required"] or values["ci_changed"]) and (is_tag or is_manual)

    expected.update({job: "success" if route.required else "skipped" for job in RUST_JOBS})
    expected[RUST_MEDIUM] = "success" if route.required and is_pr else "skipped"
    expected[CACHE_JOB] = "success" if route.warm_cache else "skipped"
    # Every non-main-push run starts the other runners' job (it classifies on
    # its own and may do nothing); a pull request only with `ci:full`.
    try:
        runner_list = json.loads(runners) if runners is not None else None
    except json.JSONDecodeError:
        runner_list = None
    allowed = PROFILE_RUNNERS.get(gate_profile or "")
    if is_main_push:
        expected[OS_JOBS] = "skipped"
    elif not isinstance(runner_list, list) or allowed is None or (
        runner_list != allowed and not (is_pr and runner_list == [LINUX])
    ):
        invalid["runners"] = (str(runners), json.dumps(allowed))
        expected[OS_JOBS] = "success"
    else:
        expected[OS_JOBS] = "success" if runner_list != [LINUX] else "skipped"
    expected.update({job: "success" if package_pipeline else "skipped" for job in PACKAGE_JOBS})
    expected[ASSESSMENT_JOB] = (
        "success" if values["assessment_required"] and (is_tag or is_manual) else "skipped"
    )
    expected[P0_PROOF_JOB] = (
        "success" if values["assessment_required"] and is_manual else "skipped"
    )
    expected["probe-thin-bootstrap"] = "success" if package_pipeline and is_manual else "skipped"
    expected.update({job: "success" if is_tag else "skipped" for job in PUBLISH_JOBS})
    if scope == "validation":
        for job in DELIVERY_ONLY:
            expected.pop(job)

    if is_tag:
        contour = "release"
    elif is_manual:
        contour = "full"
    elif is_main_push:
        contour = "cache"
    elif is_branch:
        contour = "branch"
    elif is_queue:
        contour = "queue"
    elif not is_pr:
        contour = "invalid"
    elif docs_only:
        contour = "documentation"
    elif all(values[name] for name in CLASSIFICATION_OUTPUTS) or values["ci_changed"]:
        contour = "full"
    elif values["platform_changed"]:
        contour = "platform"
    elif values["toolchain_changed"]:
        contour = "toolchain"
    elif values["rust_changed"]:
        contour = "rust"
    elif package_pipeline:
        contour = "package"
    else:
        contour = "source"

    return contour, expected, invalid


def evaluate_gate(
    event_name: str,
    ref: str,
    classification: Mapping[str, str],
    results: Mapping[str, str],
    *,
    gate_profile: str | None = None,
    python_matrix: str | None = None,
    runners: str | None = None,
    scope: str = "delivery",
) -> GateEvaluation:
    contour, expected, unexpected = expected_results(
        event_name, ref, classification,
        gate_profile=gate_profile, python_matrix=python_matrix, runners=runners, scope=scope,
    )
    unexpected = dict(unexpected)

    for job, expected_result in expected.items():
        actual_result = results.get(job, "missing")
        if actual_result != expected_result:
            unexpected[job] = (actual_result, expected_result)
    # Джоба, которой нет в таблице ожиданий, — не «ничего», а отказ: иначе
    # новая джоба в `needs` гейта падала бы незамеченной, и гейт был бы зелёным.
    for job, actual_result in results.items():
        if job not in expected:
            unexpected[job] = (actual_result, "джоба не в таблице ворот")

    skipped_jobs = [job for job in expected if results.get(job) == "skipped"]
    return GateEvaluation(
        event_name=event_name,
        ref=ref,
        classification={name: classification.get(name, "") for name in SCOPE_OUTPUTS},
        contour=contour,
        results=dict(results),
        expected=expected,
        unexpected=unexpected,
        skipped_jobs=skipped_jobs,
    )


def render_summary(evaluation: GateEvaluation) -> str:
    lines = [
        "## Unica CI",
        "",
        f"- Event: `{evaluation.event_name}`",
        f"- Contour: `{evaluation.contour}`",
        f"- Gate: `{'success' if evaluation.ok else 'failure'}`",
        "",
        "### Classification",
        "",
    ]
    for name in SCOPE_OUTPUTS:
        label = name.replace("_", " ").capitalize()
        lines.append(f"- {label}: `{evaluation.classification.get(name) or 'missing'}`")

    lines.extend(
        [
            "",
            "### Job results",
            "",
            "| Job | Result | Expected |",
            "| --- | --- | --- |",
        ]
    )
    for job, expected_result in evaluation.expected.items():
        actual_result = evaluation.results.get(job, "missing")
        lines.append(f"| `{job}` | `{actual_result}` | `{expected_result}` |")

    lines.extend(["", "### Skipped jobs", ""])
    if evaluation.skipped_jobs:
        lines.extend(f"- `{job}`" for job in evaluation.skipped_jobs)
    else:
        lines.append("- None")

    if evaluation.unexpected:
        lines.extend(["", "### Unexpected results", ""])
        for item, (actual, expected) in evaluation.unexpected.items():
            lines.append(f"- `{item}`: got `{actual}`, expected `{expected}`")

    return "\n".join(lines) + "\n"


def main() -> int:
    """`GATE_SCOPE=validation` — the `Unica CI` job over its own steps;
    `delivery` — `Delivery CI` over those results and the delivery tail."""
    scope = os.environ.get("GATE_SCOPE", "validation")
    if scope == "validation":
        steps = json.loads(os.environ["STEPS_JSON"])
        os_jobs_path = os.environ.get("OS_JOBS_FILE", "")
        os_jobs = json.loads(Path(os_jobs_path).read_text(encoding="utf-8")) if os_jobs_path and Path(os_jobs_path).is_file() else None
        classifier = steps.get("classify", {})
        outputs = classifier.get("outputs", {}) if isinstance(classifier, dict) else {}
        results = validation_results(steps, os_jobs, os.environ.get("GATE_PROFILE"))
    elif scope == "delivery":
        needs = json.loads(os.environ["NEEDS_JSON"])
        gate = needs.get("unica-ci", {})
        outputs = gate.get("outputs", {}) if isinstance(gate, dict) else {}
        try:
            results = json.loads(outputs.get("gate_results", ""))
        except (TypeError, json.JSONDecodeError):
            results = {}
        if not isinstance(results, dict):
            results = {}
        results.update({
            job: details.get("result", "missing")
            for job, details in needs.items()
            if isinstance(details, dict) and job not in ("unica-ci", "rust-os")
        })
    else:
        raise SystemExit(f"unknown GATE_SCOPE {scope!r}")
    classification = {
        name: outputs.get(name, "") if isinstance(outputs, dict) else ""
        for name in SCOPE_OUTPUTS
    }
    evaluation = evaluate_gate(
        os.environ.get("GITHUB_EVENT_NAME", ""),
        os.environ.get("GITHUB_REF", ""),
        classification,
        results,
        gate_profile=os.environ.get("GATE_PROFILE"),
        python_matrix=os.environ.get("PYTHON_MATRIX"),
        runners=os.environ.get("RUNNERS"),
        scope=scope,
    )
    summary = render_summary(evaluation)
    print(summary, end="")
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with Path(summary_path).open("a", encoding="utf-8") as stream:
            stream.write(summary)
    output_path = os.environ.get("GITHUB_OUTPUT")
    if output_path and scope == "validation":
        with Path(output_path).open("a", encoding="utf-8") as stream:
            stream.write(f"gate_results={json.dumps(results, separators=(',', ':'))}\n")
    return 0 if evaluation.ok else 1


if __name__ == "__main__":
    sys.exit(main())
