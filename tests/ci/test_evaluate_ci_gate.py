from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "evaluate-ci-gate.py"


def load_gate_module():
    if not MODULE_PATH.exists():
        return None
    spec = importlib.util.spec_from_file_location("evaluate_ci_gate", MODULE_PATH)
    if spec is None or spec.loader is None:
        return None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


COMMON_SUCCESS = {
    "classify-changes": "success",
    "verify-source": "success",
    "test-rust-platforms": "success",
}
FULL_SUCCESS = {
    "build-tools": "success",
    "package-runtime": "success",
    "package-thin": "success",
    "release-assessment": "success",
}
PUBLISH_SKIPPED = {
    "publish-release-assets": "skipped",
    "smoke-thin-plugin": "skipped",
    "verify-published-assets": "skipped",
}


class EvaluateCiGateTests(unittest.TestCase):
    def module(self):
        module = load_gate_module()
        self.assertIsNotNone(module, f"missing gate evaluator: {MODULE_PATH}")
        self.assertTrue(hasattr(module, "evaluate_gate"), "missing evaluate_gate")
        self.assertTrue(hasattr(module, "render_summary"), "missing render_summary")
        return module

    def test_light_pr_accepts_only_classified_pipeline_skips(self) -> None:
        module = self.module()
        results = {
            **COMMON_SUCCESS,
            "build-tools": "skipped",
            "package-runtime": "skipped",
            "package-thin": "skipped",
            "probe-thin-bootstrap": "skipped",
            "release-assessment": "skipped",
            **PUBLISH_SKIPPED,
        }

        evaluation = module.evaluate_gate("pull_request", "refs/pull/148/merge", "false", results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("light", evaluation.contour)
        self.assertEqual(set(results) - set(COMMON_SUCCESS), set(evaluation.skipped_jobs))
        self.assertEqual({}, evaluation.unexpected)

    def test_full_pr_requires_the_conditional_pipeline(self) -> None:
        module = self.module()
        results = {
            **COMMON_SUCCESS,
            **FULL_SUCCESS,
            "probe-thin-bootstrap": "success",
            **PUBLISH_SKIPPED,
        }

        evaluation = module.evaluate_gate("pull_request", "refs/pull/148/merge", "true", results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("full", evaluation.contour)
        self.assertEqual(set(PUBLISH_SKIPPED), set(evaluation.skipped_jobs))

    def test_failure_cancelled_and_unexpected_skip_fail_the_gate(self) -> None:
        module = self.module()
        results = {
            **COMMON_SUCCESS,
            **FULL_SUCCESS,
            "probe-thin-bootstrap": "success",
            **PUBLISH_SKIPPED,
            "verify-source": "cancelled",
            "build-tools": "failure",
            "package-runtime": "skipped",
        }

        evaluation = module.evaluate_gate("pull_request", "refs/pull/148/merge", "true", results)

        self.assertFalse(evaluation.ok)
        self.assertEqual(
            {
                "verify-source": ("cancelled", "success"),
                "build-tools": ("failure", "success"),
                "package-runtime": ("skipped", "success"),
            },
            evaluation.unexpected,
        )

    def test_tag_gate_requires_publication_but_skips_pr_probe(self) -> None:
        module = self.module()
        results = {
            **COMMON_SUCCESS,
            **FULL_SUCCESS,
            "probe-thin-bootstrap": "skipped",
            "publish-release-assets": "success",
            "smoke-thin-plugin": "success",
            "verify-published-assets": "success",
        }

        evaluation = module.evaluate_gate("push", "refs/tags/v0.8.1", "true", results)

        self.assertTrue(evaluation.ok)
        self.assertEqual("release", evaluation.contour)
        self.assertEqual(["probe-thin-bootstrap"], evaluation.skipped_jobs)

    def test_invalid_classifier_output_fails_closed(self) -> None:
        module = self.module()
        results = {**COMMON_SUCCESS, **FULL_SUCCESS, "probe-thin-bootstrap": "success", **PUBLISH_SKIPPED}

        evaluation = module.evaluate_gate("pull_request", "refs/pull/148/merge", "", results)

        self.assertFalse(evaluation.ok)
        self.assertEqual("invalid", evaluation.contour)
        self.assertIn("classification", evaluation.unexpected)

    def test_summary_reports_contour_results_and_skipped_jobs(self) -> None:
        module = self.module()
        results = {
            **COMMON_SUCCESS,
            **FULL_SUCCESS,
            "probe-thin-bootstrap": "success",
            **PUBLISH_SKIPPED,
        }
        evaluation = module.evaluate_gate("pull_request", "refs/pull/148/merge", "true", results)

        summary = module.render_summary(evaluation)

        self.assertIn("Contour: `full`", summary)
        self.assertIn("| `publish-release-assets` | `skipped` | `skipped` |", summary)
        self.assertIn("Skipped jobs", summary)
        self.assertIn("`verify-published-assets`", summary)


if __name__ == "__main__":
    unittest.main()
