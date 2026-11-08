"""Пересборка линии из сохранённых результатов не удваивает историю."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "build-site.py"


def load_module():
    spec = importlib.util.spec_from_file_location("build_site", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class UnwindHistoryTests(unittest.TestCase):
    def test_unwind_drops_the_rebuilt_launch_from_trends_and_test_history(self) -> None:
        """Вершина истории — пересобираемый прогон; Allure положит его заново."""
        module = load_module()
        history = Path(tempfile.mkdtemp(prefix="history-"))
        (history / "history-trend.json").write_text(json.dumps([{"buildOrder": 2}, {"buildOrder": 1}]), encoding="utf-8")
        (history / "duration-trend.json").write_text(json.dumps([{"buildOrder": 2}]), encoding="utf-8")
        (history / "history.json").write_text(json.dumps({
            "old": {"statistic": {"failed": 1, "passed": 1, "total": 2}, "items": [{"status": "failed"}, {"status": "passed"}]},
            "new": {"statistic": {"passed": 1, "total": 1}, "items": [{"status": "passed"}]},
        }), encoding="utf-8")

        unwound = module.unwind_history(history)

        self.assertEqual(3, unwound)
        self.assertEqual([{"buildOrder": 1}], json.loads((history / "history-trend.json").read_text(encoding="utf-8")))
        self.assertEqual([], json.loads((history / "duration-trend.json").read_text(encoding="utf-8")))
        tests = json.loads((history / "history.json").read_text(encoding="utf-8"))
        self.assertEqual({"old"}, set(tests))
        self.assertEqual({"failed": 0, "passed": 1, "total": 1}, tests["old"]["statistic"])
        self.assertEqual([{"status": "passed"}], tests["old"]["items"])


class MergeResultsTests(unittest.TestCase):
    """Отчёт линии — объединение ярусов: поздняя запись побеждает, попытки вместе."""

    def record(self, out: Path, history_id: str, stop: int, uid: str) -> None:
        out.mkdir(parents=True, exist_ok=True)
        (out / f"{uid}-result.json").write_text(json.dumps({
            "uuid": uid, "historyId": history_id, "name": history_id, "fullName": history_id,
            "status": "passed", "start": stop - 1, "stop": stop, "labels": [],
        }), encoding="utf-8")

    def test_latest_record_wins_and_retry_attempts_travel_together(self) -> None:
        module = load_module()
        root = Path(tempfile.mkdtemp(prefix="merge-"))
        fresh, stored = root / "fresh", root / "stored"
        self.record(fresh, "a", 200, "a-fresh")
        self.record(fresh, "b", 200, "b-fresh")
        (fresh / "run.json").write_text(json.dumps({"profile": "main"}), encoding="utf-8")
        self.record(stored, "a", 100, "a-old")
        self.record(stored, "c", 100, "c-try")
        self.record(stored, "c", 150, "c-final")
        (stored / "run.json").write_text(json.dumps({"profile": "large"}), encoding="utf-8")
        (stored / "executor.json").write_text("{}", encoding="utf-8")

        count = module.merge_results([fresh, stored], root / "merged")

        names = sorted(p.name for p in (root / "merged").glob("*-result.json"))
        self.assertEqual(count, 3)
        self.assertEqual(names, ["a-fresh-result.json", "b-fresh-result.json", "c-final-result.json", "c-try-result.json"])
        self.assertEqual(json.loads((root / "merged" / "run.json").read_text(encoding="utf-8"))["profile"], "main")
        self.assertTrue((root / "merged" / "executor.json").is_file())

    def test_stored_results_fall_back_to_the_legacy_archive_as_main(self) -> None:
        module = load_module()
        root = Path(tempfile.mkdtemp(prefix="stored-"))
        legacy = root / "legacy"
        self.record(legacy, "a", 1, "a")
        import tarfile
        archive = root / "legacy.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            for item in legacy.iterdir():
                bundle.add(item, arcname=item.name)

        def fetch(url: str, target: Path) -> bool:
            if url.endswith("/data/main/results.tar.gz"):
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(archive.read_bytes())
                return True
            return False

        module.fetch = fetch
        stored = module.stored_results("https://site", "main", root / "work")

        self.assertEqual(list(stored), ["main"])
        self.assertTrue((stored["main"][1] / "a-result.json").is_file())

    def test_queue_results_survive_publishing_another_profile_and_can_be_read_back(self) -> None:
        """Ночная публикация сохраняет архив очереди, его записи и подпись."""
        module = load_module()
        with tempfile.TemporaryDirectory(prefix="queue-retention-") as temporary:
            root = Path(temporary)
            queue, large = root / "queue", root / "large"
            self.record(queue, "queue-case", 100, "queue-case")
            self.record(large, "large-case", 200, "large-case")
            queue_run = {"profile": "queue", "sha": "queue-sha", "at": "2026-10-20T01:00:00Z"}
            (queue / "run.json").write_text(json.dumps(queue_run), encoding="utf-8")
            (large / "run.json").write_text(json.dumps({
                "profile": "large", "sha": "large-sha", "at": "2026-10-20T02:00:00Z",
            }), encoding="utf-8")
            published = root / "before" / "site"
            renders = []

            def fetch(url: str, target: Path) -> bool:
                source = published / url.removeprefix("https://site/")
                if not source.is_file():
                    return False
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(source.read_bytes())
                return True

            def render(command: list[str], **_kwargs) -> None:
                results = Path(command[-1])
                renders.append({
                    path.name: json.loads(path.read_text(encoding="utf-8"))
                    for path in results.glob("*-result.json")
                })

            args = Namespace(
                line=[f"main={queue}"], site=None, allure_command="allure",
                run_sha="queue-sha", run_url="https://x/runs/1", run_at=queue_run["at"],
            )
            with (
                patch.object(module, "fetch", side_effect=fetch),
                patch.object(module.shutil, "which", return_value="allure"),
                patch.object(module.subprocess, "run", side_effect=render),
            ):
                module.build_reports(published, args)
                queue_archive = (published / "data/main/results/queue.tar.gz").read_bytes()

                args.line = [f"main={large}"]
                args.site = "https://site"
                args.run_sha = "large-sha"
                args.run_url = "https://x/runs/2"
                args.run_at = "2026-10-20T02:00:00Z"
                after = root / "after" / "site"
                module.build_reports(after, args)

                retained = after / "data/main/results/queue.tar.gz"
                self.assertTrue(retained.is_file(), "публикация large потеряла архив queue")
                self.assertEqual(retained.read_bytes(), queue_archive)
                self.assertEqual(set(renders[1]), {"queue-case-result.json", "large-case-result.json"})
                self.assertEqual(renders[1]["queue-case-result.json"], renders[0]["queue-case-result.json"])

                published = after
                stored = module.stored_results("https://site", "main", root / "read-back")
                self.assertEqual(set(stored), {"queue", "large"})
                self.assertEqual(module.signature_of(stored["queue"][1]), queue_run)
                self.assertEqual(
                    (stored["queue"][1] / "queue-case-result.json").read_bytes(),
                    (queue / "queue-case-result.json").read_bytes(),
                )


class SiteProfilesTests(unittest.TestCase):
    """Профили ворот, доходящие до сайта, сайт обязан хранить и читать назад."""

    def test_every_gate_profile_that_reaches_the_site_is_stored(self) -> None:
        site = load_module()
        spec = importlib.util.spec_from_file_location("run_tests", MODULE_PATH.parent / "run-tests.py")
        run_tests = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(run_tests)

        # PR на сайт не идёт; очередь main публикуется после merge_group.
        reaching = set(run_tests.PROFILES) - {"pr"}
        self.assertTrue(reaching <= set(site.SITE_PROFILES), reaching - set(site.SITE_PROFILES))
        self.assertTrue(set(site.LARGE_PROFILES) <= reaching)


class LargeMemoryTests(unittest.TestCase):
    """Память ночного прогона живёт на сайте и пишется только прогоном large."""

    def test_only_a_large_run_writes_memory_and_others_keep_the_site_copy(self) -> None:
        module = load_module()
        root = Path(tempfile.mkdtemp(prefix="memory-"))
        results = root / "results"
        results.mkdir()
        (results / "run.json").write_text(json.dumps({
            "sha": "abc1234567", "at": "2026-09-05T01:30:00Z", "run_url": "https://x/runs/7", "run_id": "7", "profile": "large",
        }), encoding="utf-8")

        note = module.record_large_memory(root / "data", "main", results, fresh=True, site=None)

        memory = json.loads((root / "data" / "profiles" / "large.json").read_text(encoding="utf-8"))
        self.assertEqual((memory["sha"], memory["profile"]), ("abc1234567", "large"))
        self.assertIn("записана", note)

        # Тег идёт профилем `release` без Windows: памятью он быть не может,
        # иначе ночь после тега пропустила бы вершину, которую Windows не видел.
        for profile in ("main", "release"):
            (results / "run.json").write_text(json.dumps({"sha": "def", "profile": profile}), encoding="utf-8")
            note = module.record_large_memory(root / f"data-{profile}", "main", results, fresh=True, site=None)
            self.assertFalse((root / f"data-{profile}" / "profiles" / "large.json").exists())
            self.assertIn("нет", note)


class HistoryKeyTests(unittest.TestCase):
    """Ключ истории считается ровно так, как его пишет CLI 2.46."""

    def test_key_matches_what_allure_writes_for_zero_one_and_two_parameters(self) -> None:
        """Значения сняты с отчёта, собранного `allure 2.46.1` на этих же результатах."""
        module = load_module()
        self.assertEqual(
            "6537b3c2c412c480515337ab37a23f77.41e8a2ef4570710d63a48f1e44998ba9",
            module.history_key("suite::alpha", [{"name": "runner", "value": "ubuntu-latest"}]),
        )
        self.assertEqual(
            "dc3830b1fb15ec1c89901ac766fbf403.d41d8cd98f00b204e9800998ecf8427e",
            module.history_key("probe::noparams", []),
        )
        # Два параметра CLI сортирует по строке `имя:значение`, а не по порядку
        # в результате: порядок записи на ключ влиять не должен.
        forward = module.history_key(
            "probe::two",
            [{"name": "runner", "value": "ubuntu-latest"}, {"name": "os", "value": "linux"}],
        )
        backward = module.history_key(
            "probe::two",
            [{"name": "os", "value": "linux"}, {"name": "runner", "value": "ubuntu-latest"}],
        )
        self.assertEqual("ab4a29f76e544ace112ad756068bf537.8bedd3325b5fded90d32559f43a11b33", forward)
        self.assertEqual(forward, backward)


class MigrateHistoryKeysTests(unittest.TestCase):
    """Переклейка истории на ключ 2.46: без неё файл истории удваивается."""

    def results_with(self, root: Path, pairs: dict[str, str]) -> Path:
        results = root / "results"
        results.mkdir(parents=True, exist_ok=True)
        for uid, (history_id, full_name) in enumerate(pairs.items()):
            (results / f"{uid}-result.json").write_text(json.dumps({
                "uuid": str(uid), "historyId": history_id, "name": full_name, "fullName": full_name,
                "status": "passed", "parameters": [{"name": "runner", "value": "ubuntu-latest"}],
                "start": 1, "stop": 2, "labels": [],
            }), encoding="utf-8")
        return results

    def history(self, results: Path, document: dict) -> Path:
        path = results / "history" / "history.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(document), encoding="utf-8")
        return path

    def test_old_key_moves_to_the_new_one_and_a_test_that_is_gone_is_dropped(self) -> None:
        module = load_module()
        root = Path(tempfile.mkdtemp(prefix="migrate-"))
        results = self.results_with(root, {"old-alpha": "suite::alpha"})
        path = self.history(results, {
            "old-alpha": {"statistic": {"passed": 2, "total": 2}, "items": [{"status": "passed"}] * 2},
            "old-gone": {"statistic": {"passed": 1, "total": 1}, "items": [{"status": "passed"}]},
        })

        moved = module.migrate_history_keys(results)

        migrated = json.loads(path.read_text(encoding="utf-8"))
        alpha = module.history_key("suite::alpha", [{"name": "runner", "value": "ubuntu-latest"}])
        self.assertEqual(1, moved)
        self.assertEqual({alpha}, set(migrated))
        self.assertEqual(2, len(migrated[alpha]["items"]))

    def test_second_pass_changes_nothing_and_a_new_key_is_never_overwritten(self) -> None:
        module = load_module()
        root = Path(tempfile.mkdtemp(prefix="migrate-again-"))
        results = self.results_with(root, {"old-alpha": "suite::alpha"})
        alpha = module.history_key("suite::alpha", [{"name": "runner", "value": "ubuntu-latest"}])
        path = self.history(results, {
            alpha: {"statistic": {"passed": 3, "total": 3}, "items": [{"status": "passed"}] * 3},
            "old-alpha": {"statistic": {"passed": 1, "total": 1}, "items": [{"status": "passed"}]},
        })

        self.assertEqual(0, module.migrate_history_keys(results))

        kept = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual({alpha}, set(kept))
        self.assertEqual(3, len(kept[alpha]["items"]))
        self.assertEqual(0, module.migrate_history_keys(results))


class VersionTests(unittest.TestCase):
    """Тесты на сайте — по версии: отчёт и сводка лежат под тегом, а не под веткой."""

    def setUp(self) -> None:
        self.module = load_module()

    @staticmethod
    def result(directory: Path, name: str, status: str, host: str, stop: int = 1, key: str | None = None) -> None:
        directory.mkdir(parents=True, exist_ok=True)
        (directory / f"{name}-{stop}-result.json").write_text(json.dumps({
            "historyId": key or name, "status": status, "stop": stop,
            "labels": [{"name": "host", "value": host}]}), encoding="utf-8")

    def test_breakdown_counts_each_test_once_by_its_last_attempt_and_host(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            results = Path(tmp)
            self.result(results, "a", "failed", "ubuntu-latest", stop=1)
            self.result(results, "a", "passed", "ubuntu-latest", stop=2)
            self.result(results, "b", "skipped", "macos-15")
            self.result(results, "c", "broken", "windows-latest")
            found = self.module.breakdown(results)["by_host"]
        self.assertEqual(found["ubuntu-latest"], {"total": 1, "passed": 1, "failed": 0, "skipped": 0})
        self.assertEqual(found["macos-15"]["skipped"], 1)
        self.assertEqual(found["windows-latest"]["failed"], 1)

    def test_a_run_log_gives_counts_per_os_and_only_unittest_skips(self) -> None:
        log = "\n".join([
            "Rust (macos-14)\tТесты\t2026-10-26T09:39:12Z      Summary [1740.6s] 6772 tests run: 6772 passed, 21 skipped",
            "Unica CI\tТесты\t2026-10-26T09:19:18Z      Summary [643.2s] 10 tests run: 8 passed, 2 failed, 119 skipped",
            "Unica CI\tPython\t2026-10-26T09:29:41Z Ran 96 tests in 117.213s",
            "Unica CI\tPython\t2026-10-26T09:29:41Z OK (skipped=3)",
            "Build tools (win-x64)\tPython\t2026-10-26T09:55:50Z Ran 13 tests in 0.2s",
            "Build tools (win-x64)\tPython\t2026-10-26T09:55:50Z FAILED (failures=1, skipped=2)",
        ])
        self.assertEqual(self.module.log_counts(log), {
            "macos-14": {"total": 6772, "passed": 6772, "failed": 0, "skipped": 0},
            "ubuntu-latest": {"total": 106, "passed": 101, "failed": 2, "skipped": 3},
            "windows-latest": {"total": 13, "passed": 10, "failed": 1, "skipped": 2},
        })

    def args(self, **overrides) -> Namespace:
        values = {"site": "https://site.invalid", "repo": "", "allure_command": "allure", "run_sha": "a" * 40,
                  "run_url": "https://run.invalid/1", "run_at": "2026-11-08T00:00:00Z"}
        return Namespace(**{**values, **overrides})

    def generate(self, command, check):
        report = Path(command[command.index("--output") + 1])
        (report / "widgets").mkdir(parents=True)
        (report / "widgets" / "summary.json").write_text(json.dumps({"statistic": {"total": 1}}), encoding="utf-8")

    def test_fresh_results_are_stored_under_the_tag_with_report_summary_and_breakdown(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fresh = root / "fresh"
            self.result(fresh, "a", "skipped", "macos-15")
            (fresh / "run.json").write_text(json.dumps({"profile": "release"}), encoding="utf-8")
            with patch.object(self.module.shutil, "which", return_value="allure"),                  patch.object(self.module.subprocess, "run", side_effect=self.generate):
                note = self.module.publish_version(root / "site", "v1.0.0", fresh, self.args(), root / "work")
            data = root / "site" / "data" / "versions" / "v1.0.0"
            self.assertIn("свежие", note)
            self.assertTrue((data / "results.tar.gz").is_file())
            self.assertEqual(json.loads((data / "run.json").read_text(encoding="utf-8"))["sha"], "a" * 40)
            self.assertEqual(json.loads((data / "breakdown.json").read_text(encoding="utf-8"))["by_host"]["macos-15"]["skipped"], 1)
            self.assertTrue((data / "summary.json").is_file())

    def test_without_stored_results_the_run_log_is_kept_and_without_it_the_version_is_dropped(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            record = {"tag": "v0.13.0", "sha": "b" * 40, "url": "https://run.invalid/2", "by_host": {}}
            with patch.object(self.module, "fetch", return_value=False),                  patch.object(self.module, "legacy_record", return_value=record):
                note = self.module.publish_version(root / "site", "v0.13.0", None, self.args(repo="o/r"), root / "work")
            self.assertIn("журнала", note)
            legacy = root / "site" / "data" / "versions" / "v0.13.0" / "legacy.json"
            self.assertEqual(json.loads(legacy.read_text(encoding="utf-8"))["url"], "https://run.invalid/2")
            with patch.object(self.module, "fetch", return_value=False),                  patch.object(self.module, "legacy_record", return_value=None):
                self.module.publish_version(root / "site2", "v0.0.1", None, self.args(repo="o/r"), root / "work")
            self.assertFalse((root / "site2" / "data" / "versions" / "v0.0.1").exists())


if __name__ == "__main__":
    unittest.main()
