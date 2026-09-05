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
