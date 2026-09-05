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


class LargeMemoryTests(unittest.TestCase):
    """Память ночного прогона живёт на сайте и пишется прогоном large или тегом."""

    def test_large_or_release_run_writes_memory_and_others_keep_the_site_copy(self) -> None:
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

        (results / "run.json").write_text(json.dumps({"sha": "def", "profile": "main"}), encoding="utf-8")
        note = module.record_large_memory(root / "data2", "main", results, fresh=True, site=None)
        self.assertFalse((root / "data2" / "profiles" / "large.json").exists())
        self.assertIn("нет", note)


if __name__ == "__main__":
    unittest.main()
