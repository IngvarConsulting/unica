from __future__ import annotations

import importlib.util
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
    "package_changed",
    "plugin_content_changed",
    "ci_changed",
    "release_required",
)
ALWAYS_SUCCESS = {"classify-changes": "success", "verify-source": "success"}
PACKAGE_SUCCESS = {
    "build-tools": "success",
    "package-thin": "success",
    "release-assessment": "success",
}
PUBLISH_SKIPPED = {
    "publish-release-assets": "skipped",
    "smoke-thin-plugin": "skipped",
    "verify-published-assets": "skipped",
}


def classification(**enabled: bool) -> dict[str, str]:
    return {name: str(enabled.get(name, False)).lower() for name in OUTPUT_NAMES}


def source_results() -> dict[str, str]:
    return {
        **ALWAYS_SUCCESS,
        "test-rust-primary": "skipped",
        "test-rust-platforms": "skipped",
        "build-tools": "skipped",
        "package-thin": "skipped",
        "probe-thin-bootstrap": "skipped",
        "release-assessment": "skipped",
        **PUBLISH_SKIPPED,
    }


class EvaluateCiGateTests(unittest.TestCase):
    def test_source_only_pr_accepts_only_classified_skips(self) -> None:
        module = load_gate_module()
        outputs = classification(plugin_content_changed=True)
        results = source_results()

        evaluation = module.evaluate_gate("pull_request", "refs/pull/155/merge", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("source", evaluation.contour)
        self.assertEqual(set(results) - set(ALWAYS_SUCCESS), set(evaluation.skipped_jobs))

    def test_platform_independent_rust_uses_primary_macos_and_package_pipeline(self) -> None:
        module = load_gate_module()
        outputs = classification(rust_changed=True, release_required=True)
        results = {
            **source_results(),
            "test-rust-primary": "success",
            **PACKAGE_SUCCESS,
            "probe-thin-bootstrap": "success",
        }

        evaluation = module.evaluate_gate("pull_request", "refs/pull/155/merge", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("rust", evaluation.contour)
        self.assertEqual("skipped", evaluation.expected["test-rust-platforms"])

    def test_platform_rust_uses_full_matrix_instead_of_primary_job(self) -> None:
        module = load_gate_module()
        outputs = classification(rust_changed=True, platform_changed=True, release_required=True)
        results = {
            **source_results(),
            "test-rust-platforms": "success",
            **PACKAGE_SUCCESS,
            "probe-thin-bootstrap": "success",
        }

        evaluation = module.evaluate_gate("pull_request", "refs/pull/155/merge", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("platform", evaluation.contour)
        self.assertEqual("skipped", evaluation.expected["test-rust-primary"])

    def test_ci_full_pr_runs_all_validation_and_package_jobs_without_publication(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        results = {
            **source_results(),
            "test-rust-platforms": "success",
            **PACKAGE_SUCCESS,
            "probe-thin-bootstrap": "success",
        }

        evaluation = module.evaluate_gate("pull_request", "refs/pull/155/merge", outputs, results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("full", evaluation.contour)
        self.assertEqual({"test-rust-primary", *PUBLISH_SKIPPED}, set(evaluation.skipped_jobs))

    def test_manual_full_contour_runs_probe_but_tag_publishes_instead(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        manual = {
            **source_results(),
            "test-rust-platforms": "success",
            **PACKAGE_SUCCESS,
            "probe-thin-bootstrap": "success",
        }
        tag = {
            **manual,
            "probe-thin-bootstrap": "skipped",
            "publish-release-assets": "success",
            "smoke-thin-plugin": "success",
            "verify-published-assets": "success",
        }

        manual_evaluation = module.evaluate_gate("workflow_dispatch", "refs/heads/main", outputs, manual)
        tag_evaluation = module.evaluate_gate("push", "refs/tags/v0.8.1", outputs, tag)

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
            **PACKAGE_SUCCESS,
            "probe-thin-bootstrap": "success",
        }

        evaluation = module.evaluate_gate(
            "workflow_dispatch",
            "refs/tags/v0.8.1",
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
                evaluation = module.evaluate_gate(
                    "pull_request", "refs/pull/155/merge", outputs, source_results()
                )
                self.assertFalse(evaluation.ok)
                self.assertIn("classification", evaluation.unexpected)

    def test_failure_cancelled_and_unexpected_skip_fail_the_gate(self) -> None:
        module = load_gate_module()
        outputs = classification(**{name: True for name in OUTPUT_NAMES})
        results = {
            **source_results(),
            "verify-source": "cancelled",
            "test-rust-platforms": "failure",
            **PACKAGE_SUCCESS,
            "package-thin": "skipped",
            "probe-thin-bootstrap": "success",
        }

        evaluation = module.evaluate_gate("pull_request", "refs/pull/155/merge", outputs, results)

        self.assertFalse(evaluation.ok)
        self.assertEqual(
            {
                "verify-source": ("cancelled", "success"),
                "test-rust-platforms": ("failure", "success"),
                "package-thin": ("skipped", "success"),
            },
            {key: value for key, value in evaluation.unexpected.items() if key != "classification"},
        )

    def test_summary_reports_classification_results_and_skipped_jobs(self) -> None:
        module = load_gate_module()
        outputs = classification(rust_changed=True, release_required=True)
        results = {
            **source_results(),
            "test-rust-primary": "success",
            **PACKAGE_SUCCESS,
            "probe-thin-bootstrap": "success",
        }
        evaluation = module.evaluate_gate("pull_request", "refs/pull/155/merge", outputs, results)

        summary = module.render_summary(evaluation)

        self.assertIn("Contour: `rust`", summary)
        self.assertIn("Rust changed: `true`", summary)
        self.assertIn("Platform changed: `false`", summary)
        self.assertIn("| `test-rust-platforms` | `skipped` | `skipped` |", summary)
        self.assertIn("Skipped jobs", summary)


if __name__ == "__main__":
    unittest.main()
