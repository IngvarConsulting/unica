from __future__ import annotations

import importlib.util
import json
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "evaluate-ci-gate.py"


def load_gate_module():
    spec = importlib.util.spec_from_file_location("evaluate_ci_gate", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {MODULE_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


OUTPUT_NAMES = (
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
ALWAYS_SUCCESS = {"classify-changes": "success", "guards": "success", "test-python": "success"}
PACKAGE_SUCCESS = {
    "build-tools": "success",
    "package-thin": "success",
}
ASSESSMENT_SUCCESS = {"release-assessment": "success"}
P0_SUCCESS = {"p0-release-proof": "success"}
PUBLISH_SKIPPED = {
    "publish-release-assets": "skipped",
    "smoke-thin-plugin": "skipped",
    "verify-published-assets": "skipped",
}


def classification(**enabled: bool) -> dict[str, str]:
    return {name: str(enabled.get(name, False)).lower() for name in (*OUTPUT_NAMES, "docs_only")}


def source_results() -> dict[str, str]:
    return {
        **ALWAYS_SUCCESS,
        "test-rust-platforms": "skipped",
        "lint-rust-platforms": "skipped",
        "warm-rust-cache": "skipped",
        "build-tools": "skipped",
        "package-thin": "skipped",
        "p0-release-proof": "skipped",
        "probe-thin-bootstrap": "skipped",
        "release-assessment": "skipped",
        **PUBLISH_SKIPPED,
    }


DOCS_MATRIX = [
    {"suite": "tests/ci", "lane": "small", "slug": "ci-small"},
    {"suite": "tests/dev", "lane": "small", "slug": "dev-small"},
]
CODE_MATRIX = [
    {"suite": "tests/ci", "lane": "small", "slug": "ci-small"},
    {"suite": "tests/ci", "lane": "medium", "slug": "ci-medium"},
    {"suite": "tests/dev", "lane": "", "slug": "dev"},
]


def evaluate(module, event, ref, outputs, results, **kwargs):
    """Supply the workflow's unchanged fixture inputs to job-result tests."""
    profile = ("pr" if event == "pull_request" else "queue" if event == "merge_group"
               else "release" if event == "push" and ref.startswith("refs/tags/") else "main")
    kwargs.setdefault("gate_profile", profile)
    if outputs.get("docs_only") == "true":
        matrix = DOCS_MATRIX
    elif kwargs["gate_profile"] == "pr":
        matrix = [CODE_MATRIX[0], CODE_MATRIX[-1]]
    elif kwargs["gate_profile"] == "large":
        matrix = []
    elif kwargs["gate_profile"] == "release":
        matrix = [*CODE_MATRIX[:2], {"suite": "tests/ci", "lane": "large", "slug": "ci-large"}, CODE_MATRIX[-1]]
    else:
        matrix = CODE_MATRIX
    kwargs.setdefault("python_matrix", json.dumps(matrix))
    return module.evaluate_gate(event, ref, outputs, results, **kwargs)


class EvaluateCiGateTests(unittest.TestCase):
    def test_code_queue_requires_the_complete_python_matrix(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        results = {**source_results(), "test-rust-platforms": "success", "lint-rust-platforms": "success"}
        self.assertTrue(evaluate(module, "merge_group", "refs/heads/queue", outputs, results).ok)
        for matrix in (
            DOCS_MATRIX, CODE_MATRIX[:1], CODE_MATRIX[1:], CODE_MATRIX + CODE_MATRIX[:1],
            [{**CODE_MATRIX[0], "lane": "medium"}, *CODE_MATRIX[1:]], [], None,
        ):
            with self.subTest(matrix=matrix):
                result = evaluate(module, "merge_group", "refs/heads/queue", outputs, results,
                                  python_matrix=json.dumps(matrix) if matrix is not None else None)
                self.assertIn("python_matrix", result.unexpected)

    def test_gate_rejects_missing_profile_and_matrix(self) -> None:
        module = load_gate_module()
        result = module.evaluate_gate("pull_request", "refs/pull/1/merge", classification(), source_results())
        self.assertIn("gate_profile", result.unexpected)
        self.assertIn("python_matrix", result.unexpected)

    def test_docs_only_queue_accepts_explicit_rust_skips_but_keeps_python_and_guards(self) -> None:
        module = load_gate_module()
        outputs = classification(docs_only=True)
        result = evaluate(module,
            "merge_group", "refs/heads/gh-readonly-queue/main/pr-1", outputs,
            source_results(), gate_profile="queue", python_matrix=json.dumps(DOCS_MATRIX),
        )
        self.assertTrue(result.ok, result.unexpected)
        for job in ("test-rust-platforms", "lint-rust-platforms", "warm-rust-cache"):
            self.assertEqual("skipped", result.expected[job])
        for job in ("guards", "test-python"):
            failed = evaluate(module,
                "merge_group", "refs/heads/gh-readonly-queue/main/pr-1", outputs,
                {**source_results(), job: "skipped"},
            )
            self.assertFalse(failed.ok)
            self.assertIn(job, failed.unexpected)

    def test_documentation_exception_requires_typed_consistent_scope_and_event(self) -> None:
        module = load_gate_module()
        for outputs in (
            {key: value for key, value in classification().items() if key != "docs_only"},
            {**classification(), "docs_only": True},
            classification(docs_only=True, ci_changed=True),
            classification(docs_only=True, rust_changed=True, release_required=True),
        ):
            with self.subTest(outputs=outputs):
                self.assertFalse(evaluate(module, "merge_group", "refs/heads/queue", outputs, source_results()).ok)
        for event, ref in (("push", "refs/heads/main"), ("push", "refs/tags/v1.0.0"), ("workflow_dispatch", "refs/heads/main")):
            result = evaluate(module, event, ref, classification(docs_only=True), source_results())
            self.assertFalse(result.ok)
            self.assertIn("docs_only", result.unexpected)

    def test_static_analysis_is_required_on_code_queue_and_every_release_contour(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        code = {**source_results(), "test-rust-platforms": "success", "lint-rust-platforms": "success"}
        manual = {**code, **PACKAGE_SUCCESS, **ASSESSMENT_SUCCESS, **P0_SUCCESS, "probe-thin-bootstrap": "success"}
        tag = {**code, **PACKAGE_SUCCESS, **ASSESSMENT_SUCCESS,
               **{job: "success" for job in PUBLISH_SKIPPED}}
        for event, ref, results in (
            ("merge_group", "refs/heads/queue", code),
            ("workflow_dispatch", "refs/heads/main", manual),
            ("push", "refs/tags/v1.0.0", tag),
        ):
            for outcome in ("failure", "cancelled", "skipped", "missing"):
                with self.subTest(event=event, outcome=outcome):
                    actual = {**results, "lint-rust-platforms": outcome}
                    if outcome == "missing":
                        del actual["lint-rust-platforms"]
                    result = evaluate(module, event, ref, outputs, actual)
                    self.assertFalse(result.ok)
                    self.assertEqual((outcome, "success"), result.unexpected["lint-rust-platforms"])

    def test_queue_cannot_be_checked_with_pr_profile(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        result = evaluate(module, "merge_group", "refs/heads/queue", outputs,
            {**source_results(), "test-rust-platforms": "success", "lint-rust-platforms": "success"},
            gate_profile="pr")
        self.assertFalse(result.ok)
        self.assertIn("gate_profile", result.unexpected)

    def test_main_push_does_not_require_repeating_tests_for_ci_or_toolchain(self) -> None:
        module = load_gate_module()
        for changed in ("ci_changed", "toolchain_changed"):
            with self.subTest(changed=changed):
                flags = {changed: True}
                if changed == "toolchain_changed":
                    flags.update(rust_changed=True, package_changed=True, release_required=True)
                outputs = classification(**flags)
                _, expected, _ = module.expected_results("push", "refs/heads/main", outputs)
                self.assertEqual("skipped", expected["test-rust-platforms"])
                self.assertEqual("skipped", expected["test-python"])

    def test_only_manual_large_with_empty_python_matrix_may_skip_python(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        results = {
            **source_results(),
            "test-python": "skipped",
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
            **PACKAGE_SUCCESS,
            **ASSESSMENT_SUCCESS,
            **P0_SUCCESS,
            "probe-thin-bootstrap": "success",
        }

        large = evaluate(module,
            "workflow_dispatch", "refs/heads/main", outputs, results,
            gate_profile="large", python_matrix="[]",
        )
        self.assertTrue(large.ok, large.unexpected)
        self.assertEqual("skipped", large.expected["test-python"])

        for event, ref, profile, matrix in (
            ("workflow_dispatch", "refs/heads/main", "main", "[]"),
            ("workflow_dispatch", "refs/heads/main", "large", '[{"suite":"ci"}]'),
            ("workflow_dispatch", "refs/heads/main", "large", None),
            ("push", "refs/heads/release-v0.13", "large", "[]"),
            ("pull_request", "refs/pull/1007/merge", "pr", "[]"),
            ("merge_group", "refs/heads/gh-readonly-queue/main/pr-1007", "queue", "[]"),
        ):
            with self.subTest(event=event, profile=profile, matrix=matrix):
                candidate = evaluate(module,
                    event, ref, outputs, results,
                    gate_profile=profile, python_matrix=matrix,
                )
                self.assertFalse(candidate.ok)
                self.assertIn("test-python", candidate.unexpected)
                if matrix == "[]" and profile != "large":
                    self.assertIn("python_matrix", candidate.unexpected)

        failed_python = evaluate(module,
            "workflow_dispatch", "refs/heads/main", outputs,
            {**results, "test-python": "failure"},
            gate_profile="large", python_matrix="[]",
        )
        self.assertFalse(failed_python.ok)
        self.assertEqual(("failure", "skipped"), failed_python.unexpected["test-python"])

        skipped_build = evaluate(module,
            "workflow_dispatch", "refs/heads/main", outputs,
            {**results, "build-tools": "skipped"},
            gate_profile="large", python_matrix="[]",
        )
        self.assertFalse(skipped_build.ok)
        self.assertEqual(("skipped", "success"), skipped_build.unexpected["build-tools"])

    def test_pull_request_skips_package_pipeline_even_when_classified(self) -> None:
        """Тяжёлые контуры сняты с pull request, а не выключены везде.

        Классификация продолжает честно говорить, что правка задела упаковку и
        оценку; гейт при этом ждёт от них пропуска, потому что на pull request
        они больше не запускаются.
        """
        module = load_gate_module()
        outputs = classification(
            package_changed=True,
            release_required=True,
            assessment_required=True,
        )

        evaluation = evaluate(module,
            "pull_request", "refs/pull/581/merge", outputs, source_results()
        )

        self.assertTrue(evaluation.ok)
        for job in ("build-tools", "package-thin", "probe-thin-bootstrap"):
            self.assertEqual("skipped", evaluation.expected[job], job)
        self.assertEqual("skipped", evaluation.expected["release-assessment"])
        self.assertEqual("skipped", evaluation.expected["p0-release-proof"])

    def test_source_only_pr_accepts_only_classified_skips(self) -> None:
        module = load_gate_module()
        outputs = classification(plugin_content_changed=True)
        results = source_results()

        evaluation = evaluate(module, "pull_request", "refs/pull/155/merge", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("source", evaluation.contour)
        self.assertEqual(set(results) - set(ALWAYS_SUCCESS), set(evaluation.skipped_jobs))

    def test_rust_only_change_runs_the_full_matrix_without_package_pipeline(self) -> None:
        """Любая правка Rust — обе платформы: одного раннера класс `#[cfg]` не видит."""
        module = load_gate_module()
        outputs = classification(rust_changed=True, release_required=True)
        results = {
            **source_results(),
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
        }

        evaluation = evaluate(module, "pull_request", "refs/pull/155/merge", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("rust", evaluation.contour)
        self.assertEqual("success", evaluation.expected["test-rust-platforms"])

    def test_platform_change_runs_the_full_matrix(self) -> None:
        module = load_gate_module()
        outputs = classification(rust_changed=True, platform_changed=True, release_required=True)
        results = {
            **source_results(),
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
        }

        evaluation = evaluate(module, "pull_request", "refs/pull/155/merge", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("platform", evaluation.contour)

    def test_long_assessment_runs_outside_pull_request_only(self) -> None:
        """Оценка на BSP осталась у ручного запуска и тега.

        На pull request её нет ни при какой классификации: это и есть снятие
        тяжёлого контура, а не отключение оценки вообще.
        """
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        manual_results = {
            **source_results(),
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
            **PACKAGE_SUCCESS,
            **ASSESSMENT_SUCCESS,
            **P0_SUCCESS,
            "probe-thin-bootstrap": "success",
        }

        manual = evaluate(module,
            "workflow_dispatch", "refs/heads/main", outputs, manual_results
        )
        request = evaluate(module,
            "pull_request",
            "refs/pull/155/merge",
            outputs,
            {
                **source_results(),
                "test-rust-platforms": "success",
                "lint-rust-platforms": "success",
            },
        )

        self.assertTrue(manual.ok)
        self.assertEqual("success", manual.expected["release-assessment"])
        self.assertTrue(request.ok)
        self.assertEqual("skipped", request.expected["release-assessment"])

    def test_ci_full_pr_runs_validation_but_no_package_jobs(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        results = {
            **source_results(),
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
        }

        evaluation = evaluate(module, "pull_request", "refs/pull/155/merge", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("full", evaluation.contour)
        self.assertEqual(
            {
                "build-tools",
                "package-thin",
                "probe-thin-bootstrap",
                "release-assessment",
                "p0-release-proof",
                "warm-rust-cache",
                *PUBLISH_SKIPPED,
            },
            set(evaluation.skipped_jobs),
        )

    def test_manual_full_contour_runs_probe_but_tag_publishes_instead(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        manual = {
            **source_results(),
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
            **PACKAGE_SUCCESS,
            **ASSESSMENT_SUCCESS,
            **P0_SUCCESS,
            "probe-thin-bootstrap": "success",
        }
        tag = {
            **manual,
            "probe-thin-bootstrap": "skipped",
            "publish-release-assets": "success",
            "smoke-thin-plugin": "success",
            "verify-published-assets": "success",
            "p0-release-proof": "skipped",
        }

        manual_evaluation = evaluate(module, "workflow_dispatch", "refs/heads/main", outputs, manual)
        tag_evaluation = evaluate(module, "push", "refs/tags/v0.9.1", outputs, tag)

        self.assertTrue(manual_evaluation.ok)
        self.assertEqual("full", manual_evaluation.contour)
        self.assertTrue(tag_evaluation.ok)
        self.assertEqual("release", tag_evaluation.contour)

    def test_manual_dispatch_on_tag_ref_remains_non_publishing_full_contour(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        results = {
            **source_results(),
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
            **PACKAGE_SUCCESS,
            **ASSESSMENT_SUCCESS,
            **P0_SUCCESS,
            "probe-thin-bootstrap": "success",
        }

        evaluation = evaluate(module,
            "workflow_dispatch",
            "refs/tags/v0.9.1",
            outputs,
            results,
        )

        self.assertTrue(evaluation.ok)
        self.assertEqual("full", evaluation.contour)
        for job in PUBLISH_SKIPPED:
            self.assertEqual("skipped", evaluation.expected[job])

    def test_missing_invalid_or_inconsistent_classification_fails_closed(self) -> None:
        module = load_gate_module()
        invalid_cases = (
            {},
            {**classification(), "rust_changed": "maybe"},
            classification(platform_changed=True),
            classification(package_changed=True),
        )
        for outputs in invalid_cases:
            with self.subTest(outputs=outputs):
                evaluation = evaluate(module,
                    "pull_request", "refs/pull/155/merge", outputs, source_results()
                )
                self.assertFalse(evaluation.ok)
                self.assertIn("classification", evaluation.unexpected)

    def test_failure_cancelled_and_unexpected_skip_fail_the_gate(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        results = {
            **source_results(),
            "test-python": "cancelled",
            "test-rust-platforms": "failure",
            "lint-rust-platforms": "success",
            # Снятый с pull request контур, который всё-таки отработал, — тоже
            # расхождение: гейт обязан заметить и лишнюю работу.
            "build-tools": "success",
        }

        evaluation = evaluate(module, "pull_request", "refs/pull/155/merge", outputs, results)

        self.assertFalse(evaluation.ok)
        self.assertEqual(
            {
                "test-python": ("cancelled", "success"),
                "test-rust-platforms": ("failure", "success"),
                "build-tools": ("success", "skipped"),
            },
            {key: value for key, value in evaluation.unexpected.items() if key != "classification"},
        )

    def test_a_job_outside_the_gate_table_fails_the_gate_even_when_green(self) -> None:
        """Новая джоба в `needs` гейта без строки в таблице — отказ, а не молчание."""
        gate = load_gate_module()
        results = {**source_results(), "brand-new-job": "failure"}

        evaluation = evaluate(gate, "pull_request", "refs/pull/1/merge", classification(), results)

        self.assertFalse(evaluation.ok)
        self.assertEqual(evaluation.unexpected["brand-new-job"], ("failure", "джоба не в таблице ворот"))
        green = evaluate(gate, "pull_request", "refs/pull/1/merge", classification(), {**source_results(), "brand-new-job": "success"})
        self.assertFalse(green.ok)

    def test_summary_reports_classification_results_and_skipped_jobs(self) -> None:
        module = load_gate_module()
        outputs = classification(rust_changed=True, release_required=True)
        results = {
            **source_results(),
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
            **PACKAGE_SUCCESS,
            "probe-thin-bootstrap": "success",
        }
        evaluation = evaluate(module, "pull_request", "refs/pull/155/merge", outputs, results)

        summary = module.render_summary(evaluation)

        self.assertIn("Contour: `rust`", summary)
        self.assertIn("Rust changed: `true`", summary)
        self.assertIn("Platform changed: `false`", summary)
        self.assertIn("| `test-rust-platforms` | `success` | `success` |", summary)
        self.assertIn("Skipped jobs", summary)


class BranchPushGateTests(unittest.TestCase):
    """Очередь проверяет main; релизные ветки проверяются после push."""

    def test_release_line_push_runs_every_test_and_no_package_pipeline(self) -> None:
        """Релизная линия вливается без очереди: push в неё — ворота линии."""
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        results = {
            **source_results(),
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
        }

        evaluation = evaluate(module, "push", "refs/heads/release-v0.13", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("branch", evaluation.contour)
        for job in (*PACKAGE_SUCCESS, *ASSESSMENT_SUCCESS, *P0_SUCCESS, "probe-thin-bootstrap"):
            self.assertEqual("skipped", evaluation.expected[job], job)

    def test_main_push_after_the_queue_runs_no_tests(self) -> None:
        """Дерево `main` проверила очередь; push сюда ничего не решает и тестов не гоняет."""
        module = load_gate_module()
        outputs = classification(rust_changed=True, release_required=True)
        results = {**source_results(), "test-python": "skipped"}

        evaluation = evaluate(module, "push", "refs/heads/main", outputs, results)

        self.assertTrue(evaluation.ok, evaluation.unexpected)
        self.assertEqual("cache", evaluation.contour)
        self.assertEqual("skipped", evaluation.expected["test-python"])
        self.assertEqual("skipped", evaluation.expected["test-rust-platforms"])
        # Тесты, всё же прошедшие на push в `main`, — расхождение с воротами, а не бонус.
        ran_anyway = evaluate(module, "push", "refs/heads/main", outputs, source_results())
        self.assertFalse(ran_anyway.ok)
        self.assertIn("test-python", ran_anyway.unexpected)

    def test_main_push_rebuilds_the_dependency_cache_when_its_key_changes(self) -> None:
        """После смены toolchain/CI компилируются зависимости без тестового прогона."""
        module = load_gate_module()
        for name in ("toolchain_changed", "ci_changed"):
            with self.subTest(flag=name):
                enabled = {name: True}
                if name == "toolchain_changed":
                    enabled.update(rust_changed=True, package_changed=True, release_required=True)
                outputs = classification(**enabled)
                results = {**source_results(), "test-python": "skipped", "warm-rust-cache": "success"}

                evaluation = evaluate(module, "push", "refs/heads/main", outputs, results)

                self.assertTrue(evaluation.ok, evaluation.unexpected)
                self.assertEqual("cache", evaluation.contour)
                self.assertEqual("success", evaluation.expected["warm-rust-cache"])
                self.assertEqual("skipped", evaluation.expected["test-rust-platforms"])
                self.assertEqual("skipped", evaluation.expected["lint-rust-platforms"])
                self.assertEqual("skipped", evaluation.expected["test-python"])

    def test_merge_group_is_the_queue_gate_full_tests_no_package_pipeline(self) -> None:
        """Очередь слияния гоняет всё, как push в ветку; упаковка остаётся тегу."""
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        results = {
            **source_results(),
            "test-rust-platforms": "success",
            "lint-rust-platforms": "success",
        }

        evaluation = evaluate(module, "merge_group", "refs/heads/gh-readonly-queue/main/pr-738-abc", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("queue", evaluation.contour)
        for job in (*PACKAGE_SUCCESS, *ASSESSMENT_SUCCESS, *P0_SUCCESS, "probe-thin-bootstrap"):
            self.assertEqual("skipped", evaluation.expected[job], job)

    def test_branch_push_with_partial_classification_is_invalid(self) -> None:
        """На ветке отбора по файлам нет: неполная классификация — ошибка гейта."""
        module = load_gate_module()
        outputs = classification(rust_changed=True)

        evaluation = evaluate(module, "push", "refs/heads/release-v0.12", outputs, source_results())

        self.assertFalse(evaluation.ok)


if __name__ == "__main__":
    unittest.main()
