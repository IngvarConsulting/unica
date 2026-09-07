"""Критерии конвейера считаются по прогонам, а страница показывает ровно их."""

from __future__ import annotations

import importlib.util
import json
import unittest
from datetime import datetime, timezone
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[2] / "scripts" / "ci"
PAGES = Path(__file__).resolve().parents[2] / "docs" / "pages"


def load(name: str):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(event: str, branch: str, sha: str, conclusion: str, created: str, minutes: int, run_id: int) -> dict:
    start = datetime.fromisoformat(created)
    end = start.replace(minute=start.minute) + (datetime.min.replace(tzinfo=timezone.utc) - datetime.min.replace(tzinfo=timezone.utc))
    from datetime import timedelta
    end = start + timedelta(minutes=minutes)
    stamp = lambda d: d.strftime("%Y-%m-%dT%H:%M:%SZ")
    return {
        "databaseId": run_id, "event": event, "headBranch": branch, "headSha": sha, "status": "completed",
        "conclusion": conclusion, "createdAt": stamp(start), "startedAt": stamp(start), "updatedAt": stamp(end),
    }


class PipelineMetricsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load("pipeline_metrics")
        self.now = datetime(2026, 9, 8, 12, 0, tzinfo=timezone.utc)
        self.runs = [
            run("pull_request", "feature/a", "aaa", "success", "2026-09-07T10:00:00+00:00", 4, 1),
            run("pull_request", "feature/a", "aaa", "failure", "2026-09-07T09:00:00+00:00", 6, 2),
            run("pull_request", "feature/b", "bbb", "success", "2026-09-07T11:00:00+00:00", 8, 3),
            run("merge_group", "gh-readonly-queue/main/pr-1-x", "m1", "success", "2026-09-07T10:10:00+00:00", 9, 4),
            run("merge_group", "gh-readonly-queue/main/pr-2-x", "m2", "failure", "2026-09-07T11:10:00+00:00", 9, 5),
            run("push", "main", "m1", "success", "2026-09-07T10:20:00+00:00", 10, 6),
            run("push", "main", "m3", "failure", "2026-09-07T12:20:00+00:00", 10, 7),
            run("merge_group", "gh-readonly-queue/main/pr-3-x", "m3", "success", "2026-09-07T12:00:00+00:00", 9, 8),
            run("push", "main", "old", "success", "2026-08-01T10:20:00+00:00", 10, 9),
        ]
        self.pages = [
            run("workflow_run", "main", "m1", "success", "2026-09-07T10:31:00+00:00", 3, 10),
            # Прямой push-прогон сайта без результатов — не пересборка по прогону.
            run("push", "main", "m1", "success", "2026-09-07T10:20:00+00:00", 1, 11),
        ]
        self.merged = [
            {"number": 1, "createdAt": "2026-09-07T08:00:00Z", "mergedAt": "2026-09-07T10:20:00Z", "headRefName": "feature/a", "mergeCommit": {"oid": "m1"}},
            {"number": 3, "createdAt": "2026-09-07T08:00:00Z", "mergedAt": "2026-09-07T12:19:00Z", "headRefName": "feature/c", "mergeCommit": {"oid": "m3"}},
        ]

    def fake_gh(self, args: list[str]):
        joined = " ".join(args)
        if "unica-pages.yml" in joined:
            return self.pages
        if "run list" in joined:
            return self.runs
        if "pr list" in joined:
            return self.merged
        if "run view" in joined:
            return {"jobs": [{"conclusion": "success", "startedAt": "2026-09-07T10:00:00Z", "completedAt": "2026-09-07T10:05:00Z"}]}
        raise AssertionError(joined)

    def fake_fetch(self, url: str):
        if url.endswith("retry-trend.json"):
            return [{"data": {"run": 10000, "retry": 2}}, {"data": {"run": 10000, "retry": 1}}]
        if url.endswith("summary.json"):
            return {"statistic": {"failed": 1, "broken": 2}}
        return None

    def test_every_criterion_is_computed_from_runs_not_typed(self) -> None:
        document = self.module.gather("IngvarConsulting/unica", "https://site", now=self.now, gh=self.fake_gh, fetch=self.fake_fetch)
        rows = {row["metric"]: row for row in document["metrics"]}

        self.assertEqual(document["since"], "25.08.2026")
        self.assertEqual(rows["Сигнал PR, медиана и 90-й процентиль"]["value"], "6,0 мин / 8,0 мин")
        # Вливание №1: последний прогон PR feature/a в 10:00, влит в 10:20 → 20 мин; у №3 прогона PR нет.
        self.assertEqual(rows["Push → вливание, медиана"]["value"], "20,0 мин")
        # m1: push в 10:20, сайт пересобран к 10:34.
        self.assertEqual(rows["Вливание → отчёт на сайте, медиана"]["value"], "14,0 мин")
        # Прогон августа за окном не считается.
        self.assertEqual(rows["Зелёный main"]["value"], "1 из 2 (50 %)")
        # m3: очередь зелёная, main красный — ложный зелёный.
        self.assertEqual(rows["Ложный зелёный: красный main при зелёной очереди"]["value"], "1")
        self.assertEqual(rows["Повторы на 10 тысяч тестов"]["value"], "1,5")
        # aaa: красный, потом зелёный без правки; bbb — нет.
        self.assertEqual(rows["PR красный, затем зелёный без правки"]["value"], "1 из 2 (50 %)")
        self.assertEqual(rows["Выкидки из очереди"]["value"], "1 из 3 (33 %)")
        self.assertEqual(rows["Сломанные и упавшие тесты в отчёте main"]["value"], "3")
        # 8 прогонов в окне по 5 минут джоб, 2 вливания.
        self.assertEqual(rows["Минуты раннеров на одно вливание"]["value"], "20")
        self.assertEqual(rows["Вливаний в день"]["value"], "0,1")

    def test_missing_sources_show_a_dash_not_a_zero(self) -> None:
        document = self.module.gather("IngvarConsulting/unica", "", now=self.now, gh=lambda args: [] if "list" in " ".join(args) else {}, fetch=lambda url: None)
        rows = {row["metric"]: row["value"] for row in document["metrics"]}

        self.assertEqual(rows["Сигнал PR, медиана и 90-й процентиль"], "—")
        self.assertEqual(rows["Повторы на 10 тысяч тестов"], "—")
        self.assertEqual(rows["Минуты раннеров на одно вливание"], "—")
        self.assertEqual(rows["Зелёный main"], "—")

    def test_the_pipeline_page_shows_exactly_the_computed_rows(self) -> None:
        """Страница рисует ровно те поля, что считает модуль: лишнее или недостающее — отказ."""
        render = load("render-pages")
        document = self.module.gather("IngvarConsulting/unica", "https://site", now=self.now, gh=self.fake_gh, fetch=self.fake_fetch)
        template = (PAGES / "pipeline.html").read_text(encoding="utf-8")
        status = {
            "metrics": document["metrics"], "metrics_days": "14", "metrics_since": document["since"], "metrics_until": document["until"],
            "github_stars": "1", "telegram_members": "1", "generated_at": "сегодня", "generated_url": "u", "generated_sha": "s",
        }

        page = render.render(template, status)

        self.assertIn("Сигнал PR", page)
        self.assertIn("6,0 мин / 8,0 мин", page)
        self.assertNotIn("{{", page)


if __name__ == "__main__":
    unittest.main()
