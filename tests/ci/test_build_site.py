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


if __name__ == "__main__":
    unittest.main()
