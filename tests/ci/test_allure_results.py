"""Перевод результатов в формат Allure: статусы, причины, история по раннеру."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "allure_results.py"

JUNIT = """<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="unica" tests="4" failures="2" errors="0">
  <testsuite name="unica-coder" tests="4" failures="2" errors="0" skipped="1">
    <testcase name="address::resolves_catalog" classname="unica-coder" time="0.010"/>
    <testcase name="address::wrong_expectation" classname="unica-coder" time="0.020">
      <failure message="test failed">assertion `left == right` failed: нарочно</failure>
      <system-err>thread 'address::wrong_expectation' panicked</system-err>
    </testcase>
    <testcase name="daemon::index_out_of_bounds" classname="unica-coder" time="0.030">
      <failure message="test failed">index out of bounds: the len is 0 but the index is 3</failure>
    </testcase>
    <testcase name="daemon::spawns_daemon" classname="unica-coder" time="0">
      <skipped message="Skipped: test does not match the run-ignored option"/>
    </testcase>
    <testcase name="address::flaky_then_green" classname="unica-coder" time="0.050">
      <flakyFailure timestamp="2026-09-05T03:00:00+00:00" time="0.040" type="test failure">assertion `left == right` failed: раз в год
        <system-err>thread 'address::flaky_then_green' panicked</system-err>
      </flakyFailure>
    </testcase>
    <testcase name="address::always_red" classname="unica-coder" time="0.010">
      <rerunFailure timestamp="2026-09-05T03:00:00+00:00" time="0.010" type="test failure">index out of bounds: первая попытка</rerunFailure>
      <failure message="test failed">index out of bounds: вторая попытка</failure>
    </testcase>
  </testsuite>
</testsuites>
"""

RUST_SOURCE = '''
#[cfg(test)]
mod daemon {
    #[test]
    #[ignore = "daemon tier: raises a daemon process; disabled on purpose"]
    fn spawns_daemon() {}
}
'''


def load_module():
    spec = importlib.util.spec_from_file_location("allure_results", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {MODULE_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class JunitTranslationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.root = Path(tempfile.mkdtemp(prefix="allure-results-"))
        (self.root / "crates" / "unica-coder" / "src").mkdir(parents=True)
        (self.root / "crates" / "unica-coder" / "src" / "lib.rs").write_text(RUST_SOURCE, encoding="utf-8")
        self.junit = self.root / "junit.xml"
        self.junit.write_text(JUNIT, encoding="utf-8")

    def entries(self, runner: str = "ubuntu-latest") -> dict[str, dict]:
        reasons = self.module.ignore_reasons(self.root)
        records = self.module.junit_records(self.junit, runner=runner, profile="all", reasons=reasons)
        return {entry["name"]: entry for entry in records}

    def test_statuses_distinguish_product_defect_from_broken_test(self) -> None:
        """JUnit от nextest пишет `<failure>` на любую панику; различаем по тексту."""
        entries = self.entries()

        self.assertEqual(entries["address::resolves_catalog"]["status"], "passed")
        self.assertEqual(entries["address::wrong_expectation"]["status"], "failed")
        self.assertEqual(entries["daemon::index_out_of_bounds"]["status"], "broken")

    def test_ignored_test_carries_the_authors_reason_not_nextests_phrase(self) -> None:
        entries = self.entries()

        skipped = entries["daemon::spawns_daemon"]
        self.assertEqual(skipped["status"], "skipped")
        self.assertEqual(
            skipped["statusDetails"]["message"],
            "отключён автором: daemon tier: raises a daemon process; disabled on purpose",
        )

    def test_failure_keeps_the_trace_for_the_report(self) -> None:
        entries = self.entries()

        self.assertIn("panicked", entries["address::wrong_expectation"]["statusDetails"]["trace"])

    def test_labels_place_rust_tests_in_one_tree_with_python(self) -> None:
        entry = self.entries()["address::resolves_catalog"]
        labels = {label["name"]: label["value"] for label in entry["labels"]}

        self.assertEqual(labels["language"], "rust")
        self.assertEqual(labels["parentSuite"], "rust")
        self.assertEqual(labels["suite"], "unica-coder")
        self.assertEqual(labels["subSuite"], "address")
        self.assertEqual(labels["host"], "ubuntu-latest")
        self.assertEqual(entry["fullName"], "unica-coder::address::resolves_catalog")

    def test_two_runners_are_two_parameterised_cases_with_separate_histories(self) -> None:
        """Повторы Allure склеивает по `fullName` и параметрам: раннер — параметр, история — своя."""
        ubuntu = self.entries("ubuntu-latest")["address::resolves_catalog"]
        macos = self.entries("macos-14")["address::resolves_catalog"]

        self.assertEqual(ubuntu["parameters"], [{"name": "runner", "value": "ubuntu-latest"}])
        self.assertEqual(macos["parameters"], [{"name": "runner", "value": "macos-14"}])
        self.assertNotEqual(ubuntu["historyId"], macos["historyId"])
        self.assertEqual(
            ubuntu["historyId"], self.entries("ubuntu-latest")["address::resolves_catalog"]["historyId"]
        )

    def test_retried_attempts_become_records_that_allure_folds_as_retries(self) -> None:
        """Попытка — своя запись с тем же именем и параметром; итог — последняя."""
        reasons = self.module.ignore_reasons(self.root)
        records = self.module.junit_records(self.junit, runner="ubuntu-latest", profile="main", reasons=reasons)

        flaky = [r for r in records if r["fullName"] == "unica-coder::address::flaky_then_green"]
        self.assertEqual([r["status"] for r in flaky], ["failed", "passed"])
        self.assertIn("раз в год", flaky[0]["statusDetails"]["message"])
        self.assertIn("panicked", flaky[0]["statusDetails"]["trace"])
        self.assertIn("retry", [l["value"] for l in flaky[0]["labels"] if l["name"] == "tag"])
        self.assertEqual(flaky[0]["parameters"], flaky[1]["parameters"])
        self.assertEqual(flaky[0]["historyId"], flaky[1]["historyId"])
        self.assertLess(flaky[0]["stop"], flaky[1]["start"])

        red = [r for r in records if r["fullName"] == "unica-coder::address::always_red"]
        self.assertEqual([r["status"] for r in red], ["broken", "broken"])
        self.assertEqual(
            [r["statusDetails"]["message"] for r in red],
            ["index out of bounds: первая попытка", "index out of bounds: вторая попытка"],
        )

    def test_write_produces_one_uuid_named_file_per_record(self) -> None:
        out = self.root / "allure-results"
        module = self.module

        paths = [module.write(out, entry) for entry in self.entries().values()]

        self.assertEqual(len(paths), 6)
        self.assertEqual(len({path.name for path in paths}), 6)
        for path in paths:
            self.assertTrue(path.name.endswith("-result.json"))
            json.loads(path.read_text(encoding="utf-8"))

    def test_run_signature_is_written_from_the_environment(self) -> None:
        import os

        env = {
            "GITHUB_REPOSITORY": "IngvarConsulting/unica",
            "GITHUB_SHA": "abcdef0123456789",
            "GITHUB_REF_NAME": "release-v0.12",
            "GITHUB_RUN_ID": "42",
            "GITHUB_RUN_ATTEMPT": "2",
            "RUNNER_OS": "Linux",
        }
        saved = {key: os.environ.get(key) for key in env}
        os.environ.update(env)
        try:
            path = self.module.write_run(self.root / "out", profile="all", runner="ubuntu-latest", ecosystem="rust")
        finally:
            for key, value in saved.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value

        signature = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(signature["ref"], "release-v0.12")
        self.assertEqual(signature["run_attempt"], "2")
        self.assertEqual(signature["run_url"], "https://github.com/IngvarConsulting/unica/actions/runs/42")
        self.assertEqual(signature["runner"], "ubuntu-latest")


if __name__ == "__main__":
    unittest.main()
