"""Docs-only Python plans preserve exact medium inventory without running fixtures."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[2] / "scripts" / "ci"
RUNNER = SCRIPTS / "run-unittest.py"


def load_collector():
    spec = importlib.util.spec_from_file_location("collect_python_scope", SCRIPTS / "collect-results.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PythonScopeSkipTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="python-scope-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.collector = load_collector()
        self.signature = {
            "sha": "b" * 40, "ref": "main", "run_id": "42", "run_attempt": "1",
            "run_url": "https://github.com/IngvarConsulting/unica/actions/runs/42",
            "profile": "queue", "runner": "ubuntu-latest", "ecosystem": "python",
        }
        self.manifest = self.root / "sizes.toml"
        self.manifest.write_text('[medium]\nentries = ["test_scope_fixture.Medium"]\n')

    def fixture(self, suite="tests/ci", *, small_failure=False, import_failure=False):
        path = self.root / suite
        path.mkdir(parents=True, exist_ok=True)
        source = (
            "import unittest\nfrom pathlib import Path\n"
            "class Small(unittest.TestCase):\n"
            "    def test_small(self):\n"
            "        Path('small-ran').write_text('yes')\n"
            + ("        self.fail('small failure sentinel')\n" if small_failure else "")
            + "class Medium(unittest.TestCase):\n"
            "    @classmethod\n"
            "    def setUpClass(cls):\n"
            "        Path('medium-fixture-ran').write_text('violation')\n"
            "        raise AssertionError('medium fixture must never run')\n"
            "    def test_medium(self):\n"
            "        raise AssertionError('medium body must never run')\n"
        )
        if import_failure:
            source = "raise RuntimeError('discovery failure sentinel')\n" + source
        (path / "test_scope_fixture.py").write_text(source)

    def invoke(self, output, *, suite="tests/ci", plan=False, scope=True, profile="queue"):
        command = [sys.executable, str(RUNNER), "-s", suite, "--results", str(output),
                   "--runner", "ubuntu-latest", "--profile", profile,
                   "--sizes", str(self.manifest), "--admit", "small", "--lane", "small"]
        if plan:
            command.append("--plan-only")
        if scope:
            command.append("--scope-skip-medium")
        return subprocess.run(command, cwd=self.root, text=True, capture_output=True, timeout=15)

    def signed(self, name):
        output = self.root / "artifacts" / name
        output.mkdir(parents=True)
        (output / "run.json").write_text(json.dumps(self.signature))
        return output

    @staticmethod
    def records(output):
        return {entry["fullName"]: entry for path in output.glob("*-result.json")
                for entry in [json.loads(path.read_text())]}

    def collect(self, conclusion="success"):
        jobs = self.root / "jobs.json"
        jobs.write_text(json.dumps({"jobs": [{"name": "Unica CI", "conclusion": conclusion}]}))
        self.collector.collect(self.root / "artifacts", self.root / "fresh", jobs, "", "")
        return self.root / "fresh" / "main"

    def test_scope_plan_and_execution_keep_medium_skipped_without_calling_fixtures(self):
        for suite in ("tests/ci", "tests/dev"):
            with self.subTest(suite=suite):
                self.fixture(suite)
                plan = self.signed("plan-python-" + suite.split("/")[-1])
                result = self.invoke(plan, suite=suite, plan=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                cases = {case["id"]: case for case in json.loads((plan / "plan.json").read_text())}
                self.assertEqual(set(cases), {"test_scope_fixture.Small.test_small", "test_scope_fixture.Medium.test_medium"})
                medium = cases["test_scope_fixture.Medium.test_medium"]
                self.assertEqual(medium["scope_skip"], {"reason": "docs_only"})
                self.assertEqual((medium["suite"], medium["size"], medium["lane"]), (suite, "medium", "small"))
                self.assertNotIn("scope_skip", cases["test_scope_fixture.Small.test_small"])
                self.assertFalse((self.root / "small-ran").exists())
                output = self.signed("results-python-" + suite.split("/")[-1])
                result = self.invoke(output, suite=suite)
                self.assertEqual(result.returncode, 0, result.stderr)
                records = self.records(output)
                self.assertEqual(records["test_scope_fixture.Small.test_small"]["status"], "passed")
                medium = records["test_scope_fixture.Medium.test_medium"]
                self.assertEqual(medium["status"], "skipped")
                self.assertTrue(medium["statusDetails"]["message"].startswith("docs-only:"))
                self.assertEqual(self.collector.label(medium, "scope_skip"), "docs_only")
                self.assertEqual(self.collector.label(medium, "suite"), suite)
                self.assertFalse((self.root / "medium-fixture-ran").exists())
                (self.root / "small-ran").unlink()
        collected = self.collect()
        metadata = json.loads((collected / "run.json").read_text())
        self.assertEqual(metadata["sha"], self.signature["sha"])
        self.assertEqual(
            {(item["suite"], item["size"], item["inventory_count"]) for item in metadata["routing_skips"]},
            {("tests/ci", "medium", 1), ("tests/dev", "medium", 1)},
        )
        for item in metadata["routing_skips"]:
            self.assertEqual(item["inventory_sources"], [
                {"sha": self.signature["sha"], "run_url": self.signature["run_url"]},
            ])
        properties = (collected / "environment.properties").read_text()
        self.assertIn("Python\\ (tests/ci,\\ medium,\\ ubuntu-latest)=", properties)
        self.assertIn("Python\\ (tests/dev,\\ medium,\\ ubuntu-latest)=", properties)

    def test_discovery_error_never_becomes_a_successful_plan_or_scope_skip(self):
        self.fixture(import_failure=True)
        for plan in (True, False):
            with self.subTest(plan=plan):
                output = self.root / f"discovery-{plan}"
                result = self.invoke(output, plan=plan)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("discovery failure sentinel", result.stderr)
                self.assertFalse((output / "plan.json").exists())
                self.assertFalse(any(self.collector.label(record, "scope_skip") for record in self.records(output).values()))

    def test_scope_flag_is_rejected_outside_queue(self):
        self.fixture()
        for profile in ("pr", "main", "large", "release", "all"):
            with self.subTest(profile=profile):
                result = self.invoke(self.root / profile, plan=True, profile=profile)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.root / profile / "plan.json").exists())

    def test_collector_preserves_small_failure_and_scope_reason_after_successful_discovery(self):
        self.fixture(small_failure=True)
        plan = self.signed("plan-python-ci-small")
        self.assertEqual(self.invoke(plan, plan=True).returncode, 0)
        output = self.signed("results-python-ci-small")
        result = self.invoke(output)
        self.assertNotEqual(result.returncode, 0)
        records = self.records(self.collect("failure"))
        self.assertEqual(records["test_scope_fixture.Small.test_small"]["status"], "failed")
        self.assertEqual(records["test_scope_fixture.Medium.test_medium"]["status"], "skipped")
        self.assertIn("small failure sentinel", records["test_scope_fixture.Small.test_small"]["statusDetails"]["message"])
        self.assertTrue(records["test_scope_fixture.Medium.test_medium"]["statusDetails"]["message"].startswith("docs-only:"))

    def test_cancelled_small_job_fills_medium_with_scope_reason_and_small_with_infrastructure(self):
        self.fixture()
        plan = self.signed("plan-python-ci-small")
        self.assertEqual(self.invoke(plan, plan=True).returncode, 0)
        self.signed("results-python-ci-small")
        records = self.records(self.collect("cancelled"))
        self.assertTrue(records["test_scope_fixture.Medium.test_medium"]["statusDetails"]["message"].startswith("docs-only:"))
        self.assertEqual(self.collector.label(records["test_scope_fixture.Medium.test_medium"], "scope_skip"), "docs_only")
        small = records["test_scope_fixture.Small.test_small"]
        self.assertIn("раннер не дошёл: Unica CI · cancelled", small["statusDetails"]["message"])
        self.assertIsNone(self.collector.label(small, "scope_skip"))

    def test_collector_rejects_actual_execution_or_ordinary_skip_of_a_scope_skipped_case(self):
        self.fixture()
        plan = self.signed("plan-python-ci-small")
        self.assertEqual(self.invoke(plan, plan=True).returncode, 0)
        output = self.signed("results-python-ci-small")
        self.assertEqual(self.invoke(output).returncode, 0)
        path = next(path for path in output.glob("*-result.json")
                    if json.loads(path.read_text())["fullName"].endswith("Medium.test_medium"))
        original = json.loads(path.read_text())
        for status in ("passed", "failed", "broken", "skipped"):
            with self.subTest(status=status):
                conflicting = {**original, "status": status}
                if status == "skipped":
                    conflicting["labels"] = [label for label in original["labels"] if label["name"] != "scope_skip"]
                path.write_text(json.dumps(conflicting))
                with self.assertRaisesRegex(ValueError, "scope_skip"):
                    self.collect()
        path.write_text(json.dumps(original))
        self.collect()

    def test_collector_rejects_malformed_scope_plan_and_inconsistent_run_identity(self):
        self.fixture()
        plan = self.signed("plan-python-ci-small")
        self.assertEqual(self.invoke(plan, plan=True).returncode, 0)
        output = self.signed("results-python-ci-small")
        cases = json.loads((plan / "plan.json").read_text())
        index = next(index for index, case in enumerate(cases) if case["size"] == "medium")
        for patch in ({"scope_skip": None}, {"scope_skip": {"reason": "other"}},
                      {"scope_skip": {"reason": "docs_only", "extra": True}},
                      {"size": "small"}, {"lane": "medium"}, {"ecosystem": "rust"},
                      {"suite": "tests/other"}, {"id": []},
                      {"id": "unittest.loader._FailedTest.test_broken"}):
            with self.subTest(patch=patch):
                changed = list(cases)
                changed[index] = {**cases[index], **patch}
                (plan / "plan.json").write_text(json.dumps(changed))
                with self.assertRaisesRegex(ValueError, "scope_skip"):
                    self.collect()
        (plan / "plan.json").write_text(json.dumps(cases))
        for patch in ({"sha": "c" * 40}, {"profile": "pr"}, {"run_attempt": "2"}, {"run_id": "43"}):
            with self.subTest(patch=patch):
                (output / "run.json").write_text(json.dumps({**self.signature, **patch}))
                with self.assertRaisesRegex(ValueError, "scope_skip"):
                    self.collect()

    def test_collector_rejects_scope_result_without_matching_suite_and_runner_plan(self):
        self.fixture()
        output = self.signed("results-python-ci-small")
        self.assertEqual(self.invoke(output).returncode, 0)
        with self.assertRaisesRegex(ValueError, "scope_skip"):
            self.collect()
        plan = self.signed("plan-python-ci-small")
        self.assertEqual(self.invoke(plan, plan=True).returncode, 0)
        path = next(path for path in output.glob("*-result.json")
                    if json.loads(path.read_text())["fullName"].endswith("Medium.test_medium"))
        original = json.loads(path.read_text())
        for label_name, label_value in (("suite", "tests/dev"), ("host", "macos-latest"),
                                        ("size", "small"), ("profile", "pr")):
            with self.subTest(label=label_name):
                changed = {**original, "labels": [
                    {**item, "value": label_value} if item["name"] == label_name else item
                    for item in original["labels"]
                ]}
                path.write_text(json.dumps(changed))
                with self.assertRaisesRegex(ValueError, "scope_skip"):
                    self.collect()

    def test_collector_rejects_another_lane_planning_the_same_scope_skipped_case(self):
        self.fixture()
        plan = self.signed("plan-python-ci-small")
        self.assertEqual(self.invoke(plan, plan=True).returncode, 0)
        self.signed("results-python-ci-small")
        cases = json.loads((plan / "plan.json").read_text())
        medium = next(case for case in cases if case["size"] == "medium")
        conflicting = {key: value for key, value in medium.items() if key != "scope_skip"}
        conflicting["lane"] = "medium"
        extra = self.signed("plan-python-ci-medium")
        (extra / "plan.json").write_text(json.dumps([conflicting]))
        with self.assertRaisesRegex(ValueError, "scope_skip"):
            self.collect()


if __name__ == "__main__":
    unittest.main()
