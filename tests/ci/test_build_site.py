"""Пересборка линии из сохранённых результатов не удваивает историю."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


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


class SiteProfilesTests(unittest.TestCase):
    """Профили ворот, доходящие до сайта, сайт обязан хранить и читать назад."""

    def test_every_gate_profile_that_reaches_the_site_is_stored(self) -> None:
        site = load_module()
        spec = importlib.util.spec_from_file_location("run_tests", MODULE_PATH.parent / "run-tests.py")
        run_tests = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(run_tests)

        # `pr` и `queue` на сайт не идут; всё остальное сайт кладёт по профилям.
        reaching = set(run_tests.PROFILES) - {"pr", "queue"}
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


if __name__ == "__main__":
    unittest.main()
