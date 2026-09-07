"""Сложение артефактов прогона в результаты линий: линии, недошедшие, метаданные."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

import yaml


SCRIPTS = Path(__file__).resolve().parents[2] / "scripts" / "ci"
WORKFLOWS = Path(__file__).resolve().parents[2] / ".github" / "workflows"


def workflow(name: str) -> dict:
    return yaml.safe_load((WORKFLOWS / name).read_text(encoding="utf-8"))


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

    def signed(self, name: str, runner: str, ecosystem: str, records: list[dict], **run) -> Path:
        path = self.artifacts / name
        path.mkdir(parents=True)
        for entry in records:
            self.allure.write(path, entry)
        (path / "run.json").write_text(json.dumps({**self.run, "runner": runner, "ecosystem": ecosystem, **run}), encoding="utf-8")
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

    def plan(self, name: str, runner: str, cases: list[str], **run) -> Path:
        path = self.signed(name, runner, "rust", [], **run)
        (path / "plan.json").write_text(json.dumps([
            {"binary": "unica-coder", "name": case, "ignored": False} for case in cases
        ]), encoding="utf-8")
        return path

    def scenario(self, conclusion: str = "cancelled") -> tuple[dict, Path]:
        self.signed("results-rust-ubuntu-latest", "ubuntu-latest", "rust", [self.rust("address::resolves", "ubuntu-latest")])
        self.signed("results-python", "ubuntu-latest", "python", [self.python("ubuntu-latest")])
        self.plan("plan-rust-ubuntu-latest", "ubuntu-latest", ["address::resolves", "daemon::never_reached"])
        jobs = self.root / "jobs.json"
        jobs.write_text(json.dumps({"jobs": [{"name": "Rust tests (ubuntu-latest)", "conclusion": conclusion}]}), encoding="utf-8")
        out = self.root / "fresh"
        lines = self.collect.collect(self.artifacts, out, jobs, "", "https://example.invalid")
        return lines, out

    def test_line_comes_from_the_run_signature_not_the_event(self) -> None:
        lines, _ = self.scenario()

        self.assertEqual(list(lines), ["release-v0.12"])

    def test_planned_test_without_a_result_becomes_skipped_with_the_runner_outcome(self) -> None:
        lines, out = self.scenario(conclusion="cancelled")
        stats = lines["release-v0.12"]

        records = [json.loads(p.read_text(encoding="utf-8")) for p in (out / "release-v0.12").glob("*-result.json")]
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
        _, out = self.scenario()
        line = out / "release-v0.12"

        categories = json.loads((line / "categories.json").read_text(encoding="utf-8"))
        self.assertIn("Инфраструктура: раннер не дошёл", [c["name"] for c in categories])
        # Java Properties читается как ISO-8859-1: кириллица едет `\\uXXXX`.
        environment = (line / "environment.properties").read_text(encoding="ascii")
        decoded = environment.encode("ascii").decode("unicode_escape")
        self.assertIn("Ветка=release-v0.12", decoded)
        self.assertIn("Попытка=2", decoded)
        executor = json.loads((line / "executor.json").read_text(encoding="utf-8"))
        self.assertEqual(executor["buildOrder"], 42)
        self.assertEqual(executor["reportUrl"], "https://example.invalid/allure/release-v0.12")
        signature = json.loads((line / "run.json").read_text(encoding="utf-8"))
        self.assertEqual((signature["ref"], signature["profile"]), ("release-v0.12", "all"))

    def test_no_result_artifacts_is_a_refusal(self) -> None:
        self.artifacts.mkdir(parents=True)

        with self.assertRaises(SystemExit):
            self.collect.collect(self.artifacts, self.root / "fresh", None, "main", "")

    def test_fallback_line_is_used_only_when_the_signature_is_silent(self) -> None:
        self.signed("results-python", "ubuntu-latest", "python", [self.python("ubuntu-latest")], ref="")

        lines = self.collect.collect(self.artifacts, self.root / "fresh", None, "main", "")

        self.assertEqual(list(lines), ["main"])

    def test_one_run_can_carry_several_lines_each_with_its_own_plan(self) -> None:
        """Ночной прогон несёт несколько линий; планы сопоставляются по подписи."""
        for line, case in (("main", "a::one"), ("release-v0.12", "b::two")):
            self.signed(f"results-{line}-rust-ubuntu-latest", "ubuntu-latest", "rust", [self.rust(case, "ubuntu-latest")], ref=line, profile="large")
            self.plan(f"plan-{line}-rust-ubuntu-latest", "ubuntu-latest", [case, f"{line}::missing"], ref=line, profile="large")

        lines = self.collect.collect(self.artifacts, self.root / "fresh", None, "", "")

        self.assertEqual(sorted(lines), ["main", "release-v0.12"])
        for line in lines:
            self.assertEqual((lines[line]["copied"], lines[line]["filled"]), (1, 1))
            gap = next(
                json.loads(p.read_text(encoding="utf-8"))
                for p in (self.root / "fresh" / line).glob("*-result.json")
                if "missing" in p.read_text(encoding="utf-8")
            )
            self.assertEqual(gap["fullName"], f"unica-coder::{line}::missing")
            self.assertEqual(gap["status"], "skipped")

    def test_a_truncated_job_list_is_a_refusal_not_an_unknown_outcome(self) -> None:
        """Обрезанная страница джоб сделала бы исходы недошедших «unknown» молча."""
        jobs = self.root / "jobs.json"
        jobs.write_text(json.dumps({"total_count": 31, "jobs": [{"name": "Guards", "conclusion": "success"}] * 30}), encoding="utf-8")

        with self.assertRaises(SystemExit):
            self.collect.job_conclusions(jobs)

    def test_signature_without_a_plan_takes_the_composition_from_the_site(self) -> None:
        """Сборка упала раньше `nextest list`: состав — из хранимого прогона, тесты — skipped."""
        stored = self.root / "stored"
        stored.mkdir()
        for name in ("address::resolves", "daemon::never_built"):
            self.allure.write(stored, self.rust(name, "ubuntu-latest"))
        # Повтор той же записи — попытки в хранимом прогоне не удваивают состав.
        self.allure.write(stored, self.rust("daemon::never_built", "ubuntu-latest", status="failed"))
        self.allure.write(stored, self.rust("other_runner::test", "macos-14"))
        self.allure.write(stored, self.python("ubuntu-latest"))
        (stored / "run.json").write_text(json.dumps({"sha": "0123456789abcdef", "profile": "all"}), encoding="utf-8")
        archive = self.root / "all.tar.gz"
        import tarfile
        with tarfile.open(archive, "w:gz") as bundle:
            for item in sorted(stored.iterdir()):
                bundle.add(item, arcname=item.name)
        fetched: list[str] = []

        def fake_fetch(url: str, target: Path) -> bool:
            fetched.append(url)
            if url.endswith("/data/release-v0.12/results/all.tar.gz"):
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(archive.read_bytes())
                return True
            return False

        self.collect.fetch = fake_fetch
        self.signed("results-rust-ubuntu-latest", "ubuntu-latest", "rust", [self.rust("address::resolves", "ubuntu-latest")])
        self.signed("results-python", "ubuntu-latest", "python", [self.python("ubuntu-latest")])
        jobs = self.root / "jobs.json"
        jobs.write_text(json.dumps({"jobs": [{"name": "Rust tests (ubuntu-latest)", "conclusion": "failure"}]}), encoding="utf-8")
        out = self.root / "fresh"

        lines = self.collect.collect(self.artifacts, out, jobs, "", "https://example.invalid")

        self.assertEqual(fetched, ["https://example.invalid/data/release-v0.12/results/all.tar.gz"])
        self.assertEqual((lines["release-v0.12"]["copied"], lines["release-v0.12"]["filled"]), (2, 1))
        gaps = [json.loads(r.read_text(encoding="utf-8")) for r in (out / "release-v0.12").glob("*-result.json")]
        gap = next(g for g in gaps if g["fullName"] == "unica-coder::daemon::never_built")
        self.assertEqual(gap["status"], "skipped")
        self.assertIn("раннер не дошёл до плана: Rust tests (ubuntu-latest) · failure", gap["statusDetails"]["message"])
        self.assertIn("состав взят из прогона 0123456", gap["statusDetails"]["message"])
        labels = {l["name"]: l["value"] for l in gap["labels"]}
        self.assertEqual((labels["language"], labels["host"]), ("rust", "ubuntu-latest"))
        self.assertIn("infrastructure", [l["value"] for l in gap["labels"] if l["name"] == "tag"])
        self.assertEqual(gap["historyId"], self.allure.history_id("unica-coder::daemon::never_built", "ubuntu-latest"))
        # Чужой раннер и Python из хранимого прогона в состав не попадают.
        self.assertFalse(any(g["fullName"] == "unica-coder::other_runner::test" for g in gaps))

    def test_job_name_and_artifact_prefixes_match_the_workflows(self) -> None:
        """Имя Rust-джобы и шаблоны артефактов — строковая связь; переименование красит тест."""
        release = workflow("unica-plugin-release.yml")
        rust_job = release["jobs"]["test-rust-platforms"]["name"]
        self.assertEqual(rust_job.replace("${{ matrix.runner }}", "{runner}"), self.collect.RUST_JOB)

        prefixes = (self.collect.RESULTS_PREFIX, self.collect.PLAN_PREFIX)
        uploads = []
        for job_id in ("test-python", "test-rust-platforms"):
            uploads += [s["with"]["name"] for s in release["jobs"][job_id]["steps"] if "upload-artifact" in s.get("uses", "")]
        nightly = workflow("unica-nightly.yml")
        uploads += [s["with"]["name"] for s in nightly["jobs"]["lines"]["steps"] if "upload-artifact" in s.get("uses", "")]
        for name in uploads:
            with self.subTest(artifact=name):
                self.assertTrue(name.startswith(prefixes), name)
        pages = workflow("unica-pages.yml")
        pattern = next(s["with"]["pattern"] for s in pages["jobs"]["build"]["steps"] if "download-artifact" in s.get("uses", ""))
        self.assertEqual(pattern, "{" + ",".join(p.rstrip("-") for p in prefixes) + "}-*")

    def test_nested_artifacts_of_a_relayed_run_are_found_at_any_depth(self) -> None:
        """Ночь выкладывает артефакты запущенного large одним своим — вложенным."""
        nested = self.artifacts / "results-nightly" / "main"
        path = nested / "results-rust-ubuntu-latest"
        path.mkdir(parents=True)
        self.allure.write(path, self.rust("a::one", "ubuntu-latest"))
        (path / "run.json").write_text(json.dumps({**self.run, "ref": "main", "runner": "ubuntu-latest", "profile": "large"}), encoding="utf-8")
        plan = nested / "plan-rust-ubuntu-latest"
        plan.mkdir()
        (plan / "run.json").write_text(json.dumps({**self.run, "ref": "main", "runner": "ubuntu-latest", "profile": "large"}), encoding="utf-8")
        (plan / "plan.json").write_text(json.dumps([{"binary": "unica-coder", "name": "a::one", "ignored": False}]), encoding="utf-8")

        lines = self.collect.collect(self.artifacts, self.root / "fresh", None, "", "")

        self.assertEqual(list(lines), ["main"])
        self.assertEqual((lines["main"]["copied"], lines["main"]["filled"]), (1, 0))


if __name__ == "__main__":
    unittest.main()
