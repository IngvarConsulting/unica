"""Сложение артефактов прогона в результаты линии: линия, недошедшие, метаданные."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[2] / "scripts" / "ci"


def load(name: str):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CollectResultsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.allure = load("allure_results")
        self.collect = load("collect-results")
        self.root = Path(tempfile.mkdtemp(prefix="collect-"))
        self.artifacts = self.root / "artifacts"
        self.run = {
            "sha": "abcdef0123456789", "ref": "release-v0.12", "run_id": "42", "run_attempt": "2",
            "run_url": "https://github.com/IngvarConsulting/unica/actions/runs/42", "profile": "all",
        }

    def results_dir(self, name: str, runner: str, ecosystem: str, records: list[dict]) -> Path:
        path = self.artifacts / name
        path.mkdir(parents=True)
        for entry in records:
            self.allure.write(path, entry)
        (path / "run.json").write_text(json.dumps({**self.run, "runner": runner, "ecosystem": ecosystem}), encoding="utf-8")
        return path

    def rust(self, name: str, runner: str, status: str = "passed") -> dict:
        return self.allure.record(
            name=name, full_name=f"unica-coder::{name}", status=status, runner=runner,
            labels=self.allure.rust_labels("unica-coder", name, "all"), tags=("all",),
        )

    def python(self, runner: str) -> dict:
        return self.allure.record(
            name="страж", full_name="test_x.Guard.test_it", status="passed", runner=runner,
            labels={"language": "python", "parentSuite": "python", "suite": "tests/ci"},
        )

    def scenario(self, conclusion: str = "cancelled") -> tuple[str, dict, Path]:
        self.results_dir("results-rust-ubuntu-latest", "ubuntu-latest", "rust",
                         [self.rust("address::resolves", "ubuntu-latest")])
        self.results_dir("results-python", "ubuntu-latest", "python", [self.python("ubuntu-latest")])
        plan = self.artifacts / "plan-rust-ubuntu-latest"
        plan.mkdir()
        (plan / "plan.json").write_text(json.dumps([
            {"binary": "unica-coder", "name": "address::resolves", "ignored": False},
            {"binary": "unica-coder", "name": "daemon::never_reached", "ignored": False},
        ]), encoding="utf-8")
        jobs = self.root / "jobs.json"
        jobs.write_text(json.dumps({"jobs": [{"name": "Rust tests (ubuntu-latest)", "conclusion": conclusion}]}), encoding="utf-8")
        out = self.root / "fresh"
        line, stats = self.collect.collect(self.artifacts, out, jobs, "", "https://example.invalid")
        return line, stats, out / line

    def test_line_comes_from_the_run_signature_not_the_event(self) -> None:
        line, _, _ = self.scenario()

        self.assertEqual(line, "release-v0.12")

    def test_planned_test_without_a_result_becomes_skipped_with_the_runner_outcome(self) -> None:
        _, stats, out = self.scenario(conclusion="cancelled")

        records = [json.loads(p.read_text(encoding="utf-8")) for p in out.glob("*-result.json")]
        self.assertEqual(stats["copied"], 2)
        self.assertEqual(stats["filled"], 1)
        gap = next(r for r in records if r["fullName"] == "unica-coder::daemon::never_reached")
        self.assertEqual(gap["status"], "skipped")
        self.assertIn("раннер не дошёл: Rust tests (ubuntu-latest) · cancelled", gap["statusDetails"]["message"])
        self.assertIn(self.run["run_url"], gap["statusDetails"]["message"])
        tags = [l["value"] for l in gap["labels"] if l["name"] == "tag"]
        self.assertIn("infrastructure", tags)
        self.assertEqual(gap["historyId"], self.allure.history_id("unica-coder::daemon::never_reached", "ubuntu-latest"))

    def test_metadata_files_describe_the_run_for_the_report(self) -> None:
        _, _, out = self.scenario()

        categories = json.loads((out / "categories.json").read_text(encoding="utf-8"))
        self.assertIn("Инфраструктура: раннер не дошёл", [c["name"] for c in categories])
        # Java Properties читается как ISO-8859-1: кириллица едет `\\uXXXX`.
        environment = (out / "environment.properties").read_text(encoding="ascii")
        decoded = environment.encode("ascii").decode("unicode_escape")
        self.assertIn("Ветка=release-v0.12", decoded)
        self.assertIn("Попытка=2", decoded)
        executor = json.loads((out / "executor.json").read_text(encoding="utf-8"))
        self.assertEqual(executor["buildOrder"], 42)
        self.assertEqual(executor["reportUrl"], "https://example.invalid/allure/release-v0.12")

    def test_no_result_artifacts_is_a_refusal(self) -> None:
        self.artifacts.mkdir(parents=True)

        with self.assertRaises(SystemExit):
            self.collect.collect(self.artifacts, self.root / "fresh", None, "main", "")

    def test_fallback_line_is_used_only_when_the_signature_is_silent(self) -> None:
        path = self.artifacts / "results-python"
        path.mkdir(parents=True)
        self.allure.write(path, self.python("ubuntu-latest"))

        line, _ = self.collect.collect(self.artifacts, self.root / "fresh", None, "main", "")

        self.assertEqual(line, "main")


if __name__ == "__main__":
    unittest.main()
