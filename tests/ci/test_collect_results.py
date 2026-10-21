"""Сложение артефактов прогона в результаты линий: линии, недошедшие, метаданные."""

from __future__ import annotations

import importlib.util
import json
import os
import tarfile
import tempfile
import unittest
from unittest.mock import patch
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

    def test_unreached_rust_cases_preserve_explicit_size_and_legacy_classification(self) -> None:
        plan = self.root / "plan"
        plan.mkdir()
        (plan / "plan.json").write_text(json.dumps([
            {"binary": "unica-coder", "name": "private::owned", "ignored": False, "size": "medium"},
            {"binary": "unica-coder", "name": "unrelated::legacy", "ignored": False},
        ]))
        output = self.root / "output"
        self.assertEqual(self.collect.fill_gaps(plan, {**self.run, "runner": "linux", "configuration": {"lane": "ordinary"}}, set(), output, {}), 2)
        entries = [json.loads(path.read_text()) for path in output.glob("*-result.json")]
        sizes = {entry["fullName"]: next(label["value"] for label in entry["labels"] if label["name"] == "size") for entry in entries}
        self.assertEqual(sizes, {"unica-coder::private::owned": "medium", "unica-coder::unrelated::legacy": "small"})
        self.assertTrue(all(entry["status"] == "skipped" for entry in entries))

    def test_unreached_rust_plan_rejects_unknown_or_malformed_explicit_size(self) -> None:
        plan = self.root / "plan"
        plan.mkdir()
        for size in (None, "unknown", [], {}, True):
            with self.subTest(size=size):
                (plan / "plan.json").write_text(json.dumps([
                    {"binary": "unica-coder", "name": "private::owned", "size": size},
                ]))
                with self.assertRaisesRegex(ValueError, "unknown Rust plan size"):
                    self.collect.fill_gaps(plan, {**self.run, "configuration": {"lane": "ordinary"}}, set(), self.root / "output", {})

    def signed(self, name: str, runner: str, ecosystem: str, records: list[dict], **run) -> Path:
        if ecosystem == "rust" and "scope_skip" not in run:
            run = {"configuration": {"lane": "ordinary"}, **run}
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
        jobs.write_text(json.dumps({"jobs": [{"name": "Rust tests (complete inventory)", "conclusion": conclusion}]}), encoding="utf-8")
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
        self.assertIn("раннер не дошёл: Rust tests (complete inventory) · cancelled", gap["statusDetails"]["message"])
        self.assertIn(self.run["run_url"], gap["statusDetails"]["message"])
        tags = [l["value"] for l in gap["labels"] if l["name"] == "tag"]
        self.assertIn("infrastructure", tags)
        self.assertEqual(gap["historyId"], self.allure.history_id("unica-coder::daemon::never_reached", "ubuntu-latest"))

    def test_planned_python_test_without_a_result_becomes_skipped_with_the_job_outcome(self) -> None:
        """План Python работает как план Rust: недошедший тест виден, а не пропадает."""
        self.signed("results-python-ci", "ubuntu-latest", "python", [self.python("ubuntu-latest")])
        plan = self.signed("plan-python-ci", "ubuntu-latest", "python", [])
        (plan / "plan.json").write_text(json.dumps([
            {"ecosystem": "python", "id": "test_x.Guard.test_it", "name": "страж", "suite": "tests/ci", "subSuite": "Guard", "size": "small"},
            {"ecosystem": "python", "id": "test_x.Guard.test_never_reached", "name": "не дошёл", "suite": "tests/ci", "lane": "medium", "subSuite": "Guard", "size": "medium"},
        ]), encoding="utf-8")
        jobs = self.root / "jobs.json"
        jobs.write_text(json.dumps({"jobs": [{"name": "Python tests (ci-medium)", "conclusion": "cancelled"}]}), encoding="utf-8")
        out = self.root / "fresh"

        lines = self.collect.collect(self.artifacts, out, jobs, "", "https://example.invalid")

        stats = lines["release-v0.12"]
        self.assertEqual((stats["copied"], stats["filled"]), (1, 1))
        gaps = [json.loads(r.read_text(encoding="utf-8")) for r in (out / "release-v0.12").glob("*-result.json")]
        gap = next(g for g in gaps if g["fullName"] == "test_x.Guard.test_never_reached")
        self.assertEqual(gap["status"], "skipped")
        self.assertIn("раннер не дошёл: Python tests (ci-medium) · cancelled", gap["statusDetails"]["message"])
        labels = {l["name"]: l["value"] for l in gap["labels"]}
        self.assertEqual((labels["language"], labels["suite"], labels["size"]), ("python", "tests/ci", "medium"))
        self.assertEqual(gap["historyId"], self.allure.history_id("test_x.Guard.test_never_reached", "ubuntu-latest"))

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

    def test_java_properties_escape_reserved_characters_and_utf16_code_units(self) -> None:
        # Protocol fixtures, not Python unicode_escape: that decoder does not
        # parse Java property keys and missed the whitespace-key collision.
        cases = [
            (" =:#!\\", "  leading", r"\ \=\:\#\!\\=\  leading" + "\n"),
            ("tab\tform\fline\nreturn\r", "tab\tform\fline\nreturn\rslash\\",
             r"tab\tform\fline\nreturn\r=tab\tform\fline\nreturn\rslash\\" + "\n"),
            ("path", r"C:\temp\new\uface", r"path=C:\\temp\\new\\uface" + "\n"),
            ("Ключ🙂", " Значение😀",
             r"\u041a\u043b\u044e\u0447\ud83d\ude42=\ \u0417\u043d\u0430\u0447\u0435\u043d\u0438\u0435\ud83d\ude00" + "\n"),
            ("controls", "\x00\x7f", r"controls=\u0000\u007f" + "\n"),
            ("", "", "=\n"),
        ]
        for key, value, expected in cases:
            with self.subTest(key=key, value=value):
                actual = self.collect.properties_line(key, value)
                self.assertEqual(actual, expected)
                self.assertTrue(actual.isascii())

    def test_scope_skip_environment_has_distinct_escaped_keys_for_both_runners(self) -> None:
        output = self.root / "metadata"
        output.mkdir()
        self.collect.write_metadata(output, {**self.run, "routing_skips": [
            {"runner": "macos-14", "inventory_count": 6296},
            {"runner": "ubuntu-latest", "inventory_count": 6302},
        ]}, "main", ["macos-14", "ubuntu-latest"], "")

        lines = (output / "environment.properties").read_text(encoding="ascii").splitlines()
        self.assertEqual([line.split("=", 1)[0] for line in lines if line.startswith("Rust")], [
            r"Rust\ (macos-14)", r"Rust\ (ubuntu-latest)",
        ])
        self.assertIn(" 6296", next(line for line in lines if line.startswith(r"Rust\ (macos-14)=")))
        self.assertIn(" 6302", next(line for line in lines if line.startswith(r"Rust\ (ubuntu-latest)=")))

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
            entry = self.rust(name, "ubuntu-latest")
            entry["labels"].append({"name": "rust_lane", "value": "ordinary"})
            self.allure.write(stored, entry)
        # Повтор той же записи — попытки в хранимом прогоне не удваивают состав.
        entry = self.rust("daemon::never_built", "ubuntu-latest", status="failed")
        entry["labels"].append({"name": "rust_lane", "value": "ordinary"})
        self.allure.write(stored, entry)
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
        jobs.write_text(json.dumps({"jobs": [{"name": "Rust tests (complete inventory)", "conclusion": "failure"}]}), encoding="utf-8")
        out = self.root / "fresh"

        lines = self.collect.collect(self.artifacts, out, jobs, "", "https://example.invalid")

        self.assertEqual(fetched, ["https://example.invalid/data/release-v0.12/results/all.tar.gz"])
        self.assertEqual((lines["release-v0.12"]["copied"], lines["release-v0.12"]["filled"]), (2, 1))
        gaps = [json.loads(r.read_text(encoding="utf-8")) for r in (out / "release-v0.12").glob("*-result.json")]
        gap = next(g for g in gaps if g["fullName"] == "unica-coder::daemon::never_built")
        self.assertEqual(gap["status"], "skipped")
        self.assertIn("раннер не дошёл до плана: Rust tests (complete inventory) · failure", gap["statusDetails"]["message"])
        self.assertIn("состав взят из прогона 0123456", gap["statusDetails"]["message"])
        labels = {l["name"]: l["value"] for l in gap["labels"]}
        self.assertEqual((labels["language"], labels["host"]), ("rust", "ubuntu-latest"))
        self.assertIn("infrastructure", [l["value"] for l in gap["labels"] if l["name"] == "tag"])
        self.assertEqual(gap["historyId"], self.allure.history_id("unica-coder::daemon::never_built", "ubuntu-latest"))
        # Чужой раннер и Python из хранимого прогона в состав не попадают.
        self.assertFalse(any(g["fullName"] == "unica-coder::other_runner::test" for g in gaps))

    def scope_skip(self, **changes) -> dict:
        return {"reason": "docs_only", "base_sha": "a" * 40, "head_sha": "b" * 40, **changes}

    def routing_artifacts(self, marker: dict | None = None, **run) -> Path:
        self.run = {**self.run, "ref": "main", "profile": "queue", "sha": "b" * 40, **run}
        # Python читается первым: причина Rust skip не должна зависеть от порядка.
        self.signed("results-aaa-python", "ubuntu-latest", "python", [self.python("ubuntu-latest")])
        return self.signed(
            "results-ci-routing/results-rust-ubuntu-latest", "ubuntu-latest", "rust", [],
            scope_skip=self.scope_skip() if marker is None else marker,
        )

    def jobs_file(self, jobs: dict[str, str]) -> Path:
        path = self.root / "jobs.json"
        path.write_text(json.dumps({"jobs": [{"name": name, "conclusion": conclusion} for name, conclusion in jobs.items()]}))
        return path

    def archive_results(self, results: Path, name: str) -> Path:
        archive = self.root / name
        with tarfile.open(archive, "w:gz") as bundle:
            for item in sorted(results.iterdir()):
                bundle.add(item, arcname=item.name)
        return archive

    def test_consecutive_scope_skips_keep_original_inventory_provenance_and_never_pass(self) -> None:
        origin = {"sha": "a" * 40, "run_url": "https://github.com/IngvarConsulting/unica/actions/runs/10"}
        stored = self.root / "stored"
        stored.mkdir()
        self.allure.write(stored, self.rust("address::resolves", "ubuntu-latest"))
        self.allure.write(stored, self.rust("address::resolves", "ubuntu-latest", "failed"))
        self.allure.write(stored, self.rust("other_runner::test", "macos-14"))
        self.allure.write(stored, self.python("ubuntu-latest"))
        (stored / "run.json").write_text(json.dumps({**origin, "profile": "queue"}))
        archive = self.archive_results(stored, "initial.tar.gz")
        fetched = []

        def fetch(url: str, target: Path) -> bool:
            fetched.append(url)
            if url != "https://site/data/main/results/queue.tar.gz":
                return False
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.read_bytes())
            return True

        self.collect.fetch = fetch
        for index, (base, head, job) in enumerate((
            ("a", "b", "Rust tests (complete inventory)"),
            ("b", "c", "Rust tests (complete inventory)"),
        )):
            with self.subTest(skip=index + 1):
                self.artifacts = self.root / f"artifacts-{index}"
                marker = self.scope_skip(base_sha=base * 40, head_sha=head * 40)
                self.routing_artifacts(marker, sha=head * 40)
                jobs = self.jobs_file({job: "skipped"})
                output = self.root / f"fresh-{index}"

                stats = self.collect.collect(self.artifacts, output, jobs, "", "https://site")

                self.assertEqual((stats["main"]["copied"], stats["main"]["filled"]), (1, 1))
                results = [json.loads(path.read_text()) for path in (output / "main").glob("*-result.json")]
                rust = [entry for entry in results if self.collect.label(entry, "language") == "rust"]
                self.assertEqual(len(rust), 1)
                entry = rust[0]
                self.assertEqual(entry["status"], "skipped")
                self.assertEqual(entry["historyId"], self.allure.history_id("unica-coder::address::resolves", "ubuntu-latest"))
                self.assertEqual(self.collect.label(entry, "inventory_sha"), origin["sha"])
                self.assertEqual(self.collect.label(entry, "inventory_run_url"), origin["run_url"])
                self.assertEqual(self.collect.label(entry, "profile"), "queue")
                self.assertIn("docs-only", entry["statusDetails"]["message"])
                self.assertIn(origin["run_url"], entry["statusDetails"]["message"])
                self.assertNotIn("раннер не дошёл", entry["statusDetails"]["message"])
                signature = json.loads((output / "main/run.json").read_text())
                self.assertEqual(signature["sha"], head * 40)
                skip = signature["routing_skips"][0]
                self.assertEqual(skip["scope_skip"], marker)
                self.assertEqual(skip["inventory_sources"], [origin])
                self.assertEqual(skip["inventory_count"], 1)
                archive = self.archive_results(output / "main", f"skip-{index}.tar.gz")
        self.assertEqual(fetched, ["https://site/data/main/results/queue.tar.gz"] * 2)

    def test_scope_skip_without_inventory_is_metadata_without_invented_cases(self) -> None:
        self.routing_artifacts()
        self.collect.fetch = lambda _url, _target: False
        jobs = self.jobs_file({"Rust tests (complete inventory)": "skipped"})
        output = self.root / "fresh"

        stats = self.collect.collect(self.artifacts, output, jobs, "", "https://site")

        self.assertEqual((stats["main"]["copied"], stats["main"]["filled"]), (1, 0))
        entries = [json.loads(path.read_text()) for path in (output / "main").glob("*-result.json")]
        self.assertEqual([self.collect.label(entry, "language") for entry in entries], ["python"])
        signature = json.loads((output / "main/run.json").read_text())
        skip = signature["routing_skips"][0]
        self.assertEqual(skip["scope_skip"], self.scope_skip())
        self.assertEqual(skip["inventory_count"], 0)
        self.assertEqual(skip["inventory_sources"], [])
        environment = (output / "main/environment.properties").read_text(encoding="ascii")
        self.assertIn("docs-only", environment)
        self.assertIn("ubuntu-latest", environment)

    def test_scope_skip_requires_a_skipped_job_and_rejects_conflicting_runner_outcomes(self) -> None:
        self.routing_artifacts()
        for conclusions in ({}, {"Rust tests (complete inventory)": "success"}, {
            "Build Rust ordinary (ubuntu-latest)": "failure", "Rust tests (complete inventory)": "skipped",
        }, {"Rust tests (complete inventory)": "cancelled"}):
            with self.subTest(conclusions=conclusions):
                with self.assertRaisesRegex(ValueError, "scope_skip"):
                    self.collect.collect(self.artifacts, self.root / "fresh", self.jobs_file(conclusions), "", "")

    def test_scope_skip_rejects_malformed_identity_and_unsupported_scope(self) -> None:
        path = self.routing_artifacts()
        signature = json.loads((path / "run.json").read_text())
        jobs = self.jobs_file({"Rust tests (complete inventory)": "skipped"})
        invalid = [
            {"scope_skip": None}, {"scope_skip": []}, {"scope_skip": "docs_only"},
            {"scope_skip": self.scope_skip(reason="unknown")},
            {"scope_skip": {"reason": "docs_only", "head_sha": "b" * 40}},
            {"scope_skip": self.scope_skip(base_sha="short")},
            {"scope_skip": self.scope_skip(base_sha="0" * 40)},
            {"scope_skip": self.scope_skip(extra=True)},
            {"scope_skip": self.scope_skip(head_sha="c" * 40)},
            {"scope_skip": self.scope_skip(base_sha="b" * 40)},
            {"profile": "large"}, {"profile": []}, {"ecosystem": "python"}, {"runner": ""},
            {"run_id": "another-run"},
        ]
        for changes in invalid:
            with self.subTest(changes=changes):
                (path / "run.json").write_text(json.dumps({**signature, **changes}))
                with self.assertRaisesRegex(ValueError, "scope_skip"):
                    self.collect.collect(self.artifacts, self.root / "fresh", jobs, "", "")

    def test_scope_skip_rejects_actual_rust_results_or_a_plan(self) -> None:
        path = self.routing_artifacts()
        jobs = self.jobs_file({"Rust tests (complete inventory)": "skipped"})
        result = self.allure.write(path, self.rust("actually::ran", "ubuntu-latest"))
        with self.assertRaisesRegex(ValueError, "scope_skip"):
            self.collect.collect(self.artifacts, self.root / "fresh", jobs, "", "")
        result.unlink()
        # Смотрим на реальные записи, даже если artifact ошибочно назван Python.
        actual = self.signed("results-python-mislabeled", "ubuntu-latest", "python", [self.rust("actually::ran", "ubuntu-latest")])
        with self.assertRaisesRegex(ValueError, "scope_skip"):
            self.collect.collect(self.artifacts, self.root / "fresh", jobs, "", "")
        for record in actual.glob("*-result.json"):
            record.unlink()
        self.plan("plan-rust-ubuntu-latest", "ubuntu-latest", ["actually::planned"])
        with self.assertRaisesRegex(ValueError, "scope_skip"):
            self.collect.collect(self.artifacts, self.root / "fresh", jobs, "", "")

    def test_docs_only_whole_rust_skip_rejects_a_signed_producer_even_without_a_plan(self) -> None:
        self.routing_artifacts()
        self.signed("plan-started-receipt", "ubuntu-latest", "rust", [], configuration={"lane": "receipt"})
        with self.assertRaisesRegex(ValueError, "scope_skip"):
            self.collect.collect(self.artifacts, self.root / "fresh", self.jobs_file({"Rust tests (complete inventory)": "skipped"}), "", "")

    def test_job_name_and_artifact_prefixes_match_the_workflows(self) -> None:
        release = workflow("unica-plugin-release.yml")
        self.assertEqual(release["jobs"]["test-rust-platforms"]["name"], self.collect.RUST_JOB)
        self.assertEqual(self.collect.RUST_JOB, "Rust tests (complete inventory)")
        self.assertEqual(release["jobs"]["test-python"]["name"].replace("${{ matrix.slug }}", "{slug}"), self.collect.PYTHON_JOB)
        run_tests = load("run-tests")
        for profile in ("pr", "queue", "main"):
            for entry in run_tests.python_matrix(profile):
                self.assertEqual(entry["slug"], self.collect.python_slug(entry["suite"], entry["lane"]))
        prefixes = (self.collect.RESULTS_PREFIX, self.collect.PLAN_PREFIX)
        rust_uploads = []
        for lane in ("ordinary", "receipt", "vendor"):
            build = release["jobs"][f"build-rust-{lane}"]
            self.assertEqual(build["name"].replace("${{ matrix.runner }}", "{runner}"),
                             self.collect.RUST_BUILD_JOB.format(lane=lane, runner="{runner}"))
            for kind, job_id in (("plan", f"build-rust-{lane}"), ("results", f"run-rust-{lane}")):
                uploads = [step for step in release["jobs"][job_id]["steps"]
                           if "upload-artifact" in step.get("uses", "") and step["with"]["name"].startswith(kind + "-")]
                self.assertEqual(len(uploads), 1)
                upload = uploads[0]
                self.assertEqual(upload.get("if"), "always()")
                suffix = "-${{ matrix.shard }}" if kind == "results" else ""
                self.assertEqual(upload["with"]["name"], f"{kind}-rust-${{{{ matrix.runner }}}}-{lane}{suffix}")
                self.assertEqual(upload["with"]["path"], ".build/ci-lane/*.json" if kind == "plan" else ".build/results")
                rust_uploads.append(upload["with"]["name"])
        self.assertEqual(len(rust_uploads), 6)
        uploads = rust_uploads + [step["with"]["name"] for step in release["jobs"]["test-python"]["steps"] if "upload-artifact" in step.get("uses", "")]
        uploads += [step["with"]["name"] for step in workflow("unica-nightly.yml")["jobs"]["lines"]["steps"] if "upload-artifact" in step.get("uses", "")]
        self.assertTrue(all(name.startswith(prefixes) for name in uploads))
        verification = [step["with"]["name"] for step in release["jobs"]["test-rust-platforms"]["steps"] if "upload-artifact" in step.get("uses", "")]
        self.assertEqual(verification, ["ci-rust-inventory-verification", "ci-rust-inventory-${{ env.GATE_PROFILE }}"])
        pages = workflow("unica-pages.yml")
        pattern = next(step["with"]["pattern"] for step in pages["jobs"]["build"]["steps"] if "download-artifact" in step.get("uses", ""))
        self.assertEqual(pattern, "{" + ",".join(prefix.rstrip("-") for prefix in prefixes) + "}-*")

    def test_nested_artifacts_of_a_relayed_run_are_found_at_any_depth(self) -> None:
        """Ночь выкладывает артефакты запущенного large одним своим — вложенным."""
        nested = self.artifacts / "results-nightly" / "main"
        path = nested / "results-rust-ubuntu-latest"
        path.mkdir(parents=True)
        self.allure.write(path, self.rust("a::one", "ubuntu-latest"))
        (path / "run.json").write_text(json.dumps({**self.run, "ref": "main", "runner": "ubuntu-latest", "profile": "large", "ecosystem": "rust", "configuration": {"lane": "ordinary"}}), encoding="utf-8")
        plan = nested / "plan-rust-ubuntu-latest"
        plan.mkdir()
        (plan / "run.json").write_text(json.dumps({**self.run, "ref": "main", "runner": "ubuntu-latest", "profile": "large", "ecosystem": "rust", "configuration": {"lane": "ordinary"}}), encoding="utf-8")
        (plan / "plan.json").write_text(json.dumps([{"binary": "unica-coder", "name": "a::one", "ignored": False}]), encoding="utf-8")

        lines = self.collect.collect(self.artifacts, self.root / "fresh", None, "", "")

        self.assertEqual(list(lines), ["main"])
        self.assertEqual((lines["main"]["copied"], lines["main"]["filled"]), (1, 0))

    def test_current_nonarchive_producer_retains_whole_invocation_identity_without_guessing_lane(self) -> None:
        producer = load("run-tests")
        for ecosystem in ("rust", "all"):
            with self.subTest(ecosystem=ecosystem):
                self.artifacts = self.root / ("artifacts-" + ecosystem)
                path = self.artifacts / "results-current-whole"
                dispatched = []

                def controlled_commands(commands):
                    dispatched.extend(commands)
                    return 0 if ecosystem == "all" else 1

                with patch.dict(os.environ, {"GITHUB_RUN_ID": "42", "GITHUB_RUN_ATTEMPT": "2", "GITHUB_REPOSITORY": "IngvarConsulting/unica"}):
                    code = producer.execute(
                        "queue", ecosystem, path, "ubuntu-latest", run_commands=controlled_commands,
                        junit=self.root / "not-produced.xml", line=self.run["ref"], sha=self.run["sha"],
                        before_feature_build=lambda: None,
                    )
                self.assertEqual(code, 0 if ecosystem == "all" else 1)
                self.assertEqual(any("run-unittest.py" in part for command in dispatched for part in command), ecosystem == "all")
                signed = json.loads((path / "run.json").read_text())
                self.assertEqual(signed["ecosystem"], ecosystem)
                self.assertNotIn("configuration", signed)
                self.assertNotIn("inventoryScope", signed)
                originals = [self.rust("whole::case", "ubuntu-latest")]
                if ecosystem == "all":
                    originals.append(self.python("ubuntu-latest"))
                for has_result in (False, True):
                    with self.subTest(has_result=has_result):
                        if has_result:
                            for original in originals:
                                self.allure.write(path, original)
                        output = self.root / (ecosystem + ("-with-result" if has_result else "-signature-only"))
                        stats = self.collect.collect(self.artifacts, output, self.jobs_file({"Rust tests (complete inventory)": "failure"}), "", "")
                        config = json.loads((output / self.run["ref"] / "run.json").read_text())["rust_configurations"][0]
                        self.assertIsNone(config["lane"])
                        self.assertEqual(config["scope"], "whole_invocation")
                        self.assertEqual(config["inventory_status"], "unknown")
                        self.assertEqual(config["result_count"], int(has_result))
                        self.assertEqual(stats[self.run["ref"]]["copied"], len(originals) if has_result else 0)
                        entries = {entry["uuid"]: entry for entry in (json.loads(record.read_text()) for record in (output / self.run["ref"]).glob("*-result.json"))}
                        expected = {entry["uuid"]: entry for entry in originals} if has_result else {}
                        self.assertEqual(entries, expected)

    def test_actual_lane_signature_cannot_lose_its_configuration_and_become_whole(self) -> None:
        lane_producer = load("ci_rust_lanes")
        path = self.artifacts / "plan-broken-receipt"
        lane_producer.signature(path, profile="queue", host="ubuntu-latest", lane="receipt", line=self.run["ref"], sha=self.run["sha"])
        signed = json.loads((path / "run.json").read_text())
        self.assertIn("inventoryScope", signed)
        del signed["configuration"]
        (path / "run.json").write_text(json.dumps(signed))
        with self.assertRaisesRegex(ValueError, "configuration"):
            self.collect.collect(self.artifacts, self.root / "fresh", None, "", "")

    def test_plan_only_and_signature_only_configurations_survive_zero_results(self) -> None:
        self.plan("plan-rust-ordinary", "ubuntu-latest", ["never::reached"])
        self.signed("plan-rust-receipt", "ubuntu-latest", "rust", [], configuration={"lane": "receipt"})
        jobs = self.jobs_file({"Rust tests (complete inventory)": "failure", "Build Rust receipt (ubuntu-latest)": "failure"})
        output = self.root / "fresh"
        stats = self.collect.collect(self.artifacts, output, jobs, "", "")
        entries = [json.loads(path.read_text()) for path in (output / self.run["ref"]).glob("*-result.json")]
        self.assertEqual((stats[self.run["ref"]]["copied"], stats[self.run["ref"]]["filled"]), (0, 1))
        self.assertEqual(self.collect.label(entries[0], "rust_lane"), "ordinary")
        configurations = json.loads((output / self.run["ref"] / "run.json").read_text())["rust_configurations"]
        receipt = next(config for config in configurations if config["lane"] == "receipt")
        self.assertFalse(receipt["has_plan"])
        self.assertEqual((receipt["inventory_status"], receipt["inventory_count"], receipt["result_count"]), ("unknown", 0, 0))
        self.assertEqual(receipt["observed_statuses"], {})
        self.assertEqual(receipt["job"], {"name": "Build Rust receipt (ubuntu-latest)", "conclusion": "failure"})
        self.assertEqual(receipt["signature_artifacts"], ["plan-rust-receipt"])

    def test_failed_receipt_never_borrows_ordinary_or_unlabeled_prior_inventory(self) -> None:
        stored = self.root / "stored"
        stored.mkdir()
        ordinary = self.rust("stored::ordinary", "ubuntu-latest")
        ordinary["labels"].append({"name": "rust_lane", "value": "ordinary"})
        self.allure.write(stored, ordinary)
        self.allure.write(stored, self.rust("stored::unknown_lane", "ubuntu-latest"))
        (stored / "run.json").write_text(json.dumps({"sha": "old-sha", "profile": "all"}))
        archive = self.archive_results(stored, "stored.tar.gz")
        def fetch(_url, target):
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.read_bytes())
            return True
        self.collect.fetch = fetch
        original = self.rust("healthy::ordinary", "ubuntu-latest")
        self.signed("results-ordinary", "ubuntu-latest", "rust", [original])
        self.plan("plan-ordinary", "ubuntu-latest", ["healthy::ordinary"])
        self.signed("plan-receipt", "ubuntu-latest", "rust", [], configuration={"lane": "receipt"})
        output = self.root / "fresh"
        self.collect.collect(self.artifacts, output, self.jobs_file({"Build Rust receipt (ubuntu-latest)": "failure"}), "", "https://site")
        entries = [json.loads(path.read_text()) for path in (output / self.run["ref"]).glob("*-result.json")]
        self.assertEqual(len(entries), 1)
        copied = entries[0]
        self.assertEqual((copied["uuid"], copied["historyId"], copied["parameters"]),
                         (original["uuid"], original["historyId"], original["parameters"]))
        self.assertEqual(self.collect.label(copied, "rust_lane"), "ordinary")
        receipt = next(config for config in json.loads((output / self.run["ref"] / "run.json").read_text())["rust_configurations"] if config["lane"] == "receipt")
        self.assertEqual((receipt["inventory_status"], receipt["result_count"]), ("unknown", 0))

    def test_same_case_in_receipt_has_its_own_seen_set_and_exact_prior_provenance(self) -> None:
        stored = self.root / "stored"
        stored.mkdir()
        entry = self.rust("shared::case", "ubuntu-latest")
        entry["labels"].append({"name": "rust_lane", "value": "receipt"})
        self.allure.write(stored, entry)
        (stored / "run.json").write_text(json.dumps({"sha": "prior-sha", "profile": "all"}))
        archive = self.archive_results(stored, "stored.tar.gz")
        def fetch(_url, target):
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.read_bytes())
            return True
        self.collect.fetch = fetch
        self.signed("results-ordinary", "ubuntu-latest", "rust", [self.rust("shared::case", "ubuntu-latest")])
        self.plan("plan-ordinary", "ubuntu-latest", ["shared::case"])
        self.signed("plan-receipt", "ubuntu-latest", "rust", [], configuration={"lane": "receipt"})
        output = self.root / "fresh"
        self.collect.collect(self.artifacts, output, self.jobs_file({"Build Rust receipt (ubuntu-latest)": "failure"}), "", "https://site")
        entries = [json.loads(path.read_text()) for path in (output / self.run["ref"]).glob("*-result.json")]
        self.assertEqual({(self.collect.label(e, "rust_lane"), e["status"]) for e in entries}, {("ordinary", "passed"), ("receipt", "skipped")})
        skipped = next(e for e in entries if e["status"] == "skipped")
        self.assertEqual(self.collect.label(skipped, "inventory_sha"), "prior-sha")
        self.assertEqual(skipped["historyId"], self.allure.history_id("unica-coder::shared::case", "ubuntu-latest"))

    def test_all_current_attempts_keep_uuid_history_runner_and_status_with_one_signed_lane(self) -> None:
        attempts = [self.rust("retried::case", "ubuntu-latest", status) for status in ("failed", "passed")]
        self.signed("results-vendor", "ubuntu-latest", "rust", attempts, configuration={"lane": "vendor"})
        self.plan("plan-vendor", "ubuntu-latest", ["retried::case"], configuration={"lane": "vendor"})
        output = self.root / "fresh"
        stats = self.collect.collect(self.artifacts, output, None, "", "")
        copied = {entry["uuid"]: entry for entry in (json.loads(path.read_text()) for path in (output / self.run["ref"]).glob("*-result.json"))}
        self.assertEqual((stats[self.run["ref"]]["copied"], stats[self.run["ref"]]["filled"]), (2, 0))
        for original in attempts:
            expected = {**original, "labels": [*original["labels"], {"name": "rust_lane", "value": "vendor"}]}
            self.assertEqual(copied[original["uuid"]], expected)
        config = json.loads((output / self.run["ref"] / "run.json").read_text())["rust_configurations"][0]
        self.assertEqual(config["observed_statuses"], {"failed": 1, "passed": 1})
        self.assertEqual((config["has_plan"], config["inventory_count"], config["result_count"]), (True, 1, 2))

    def test_signed_configuration_and_common_identity_conflicts_are_rejected(self) -> None:
        original = self.rust("owned::case", "ubuntu-latest")
        original["labels"].append({"name": "rust_lane", "value": "receipt"})
        path = self.signed("results-ordinary", "ubuntu-latest", "rust", [original])
        with self.assertRaisesRegex(ValueError, "lane"):
            self.collect.collect(self.artifacts, self.root / "fresh", None, "", "")
        for record in path.glob("*-result.json"):
            record.unlink()
        for field in ("sha", "profile", "run_id", "run_attempt"):
            with self.subTest(field=field):
                other = self.signed("plan-conflict-" + field, "ubuntu-latest", "rust", [], **{field: "another"})
                with self.assertRaisesRegex(ValueError, "identity"):
                    self.collect.collect(self.artifacts, self.root / "fresh", None, "", "")
                (other / "run.json").unlink()
        signed = json.loads((path / "run.json").read_text())
        for configuration in ({"lane": "unknown"}, None):
            (path / "run.json").write_text(json.dumps({**signed, "configuration": configuration}))
            with self.assertRaisesRegex(ValueError, "configuration"):
                self.collect.collect(self.artifacts, self.root / "fresh", None, "", "")



if __name__ == "__main__":
    unittest.main()
