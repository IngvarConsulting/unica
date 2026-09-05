"""Ночной прогон идёт только туда, где вершина линии сдвинулась, — и на саму линию."""

from __future__ import annotations

import importlib.util
import unittest
from datetime import datetime, timezone
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "nightly-lines.py"


def load_module():
    spec = importlib.util.spec_from_file_location("nightly_lines", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class NightlyLinesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.heads = {"main": "aaaa111" + "0" * 33, "release-v0.12": "bbbb222" + "0" * 33, "release-v0.13": "dddd444" + "0" * 33}
        self.memory = {
            "https://site/data/main/profiles/large.json": {"sha": self.heads["main"], "at": "2026-09-04T01:30:00Z"},
            "https://site/data/release-v0.13/profiles/large.json": {"sha": "cccc333" + "0" * 33, "at": "2026-08-30T01:30:00Z"},
        }

    def decisions(self, memory=None, platform=None):
        memory = self.memory if memory is None else memory
        platform = platform or (lambda line: line != "release-v0.12")
        return self.module.enumerate_lines(
            "IngvarConsulting/unica",
            "https://site",
            datetime(2026, 9, 5, tzinfo=timezone.utc),
            gh=lambda repo, path: {"commit": {"sha": self.heads[path.removeprefix("branches/")]}},
            fetch=lambda url: memory.get(url),
            open_lines=lambda repo, now: ["release-v0.12", "release-v0.13"],
            platform=platform,
        )

    def test_unmoved_line_is_skipped_and_moved_line_runs(self) -> None:
        decisions = {d["line"]: d for d in self.decisions()}

        self.assertFalse(decisions["main"]["run"])
        self.assertIn("вершина на месте", decisions["main"]["reason"])
        self.assertTrue(decisions["release-v0.13"]["run"])
        self.assertIn("cccc333 → dddd444", decisions["release-v0.13"]["reason"])

    def test_line_without_the_platform_is_skipped_with_its_reason(self) -> None:
        """Старая релизная линия не несёт unica-large.yml — запускать на ней нечего."""
        decisions = {d["line"]: d for d in self.decisions()}

        self.assertFalse(decisions["release-v0.12"]["run"])
        self.assertIn("без площадки", decisions["release-v0.12"]["reason"])

    def test_line_without_memory_runs(self) -> None:
        decisions = {d["line"]: d for d in self.decisions(memory={})}

        self.assertTrue(decisions["main"]["run"])
        self.assertIn("памяти", decisions["main"]["reason"])

    def test_dispatch_starts_large_on_each_moved_line_only(self) -> None:
        started = []

        launched = self.module.dispatch(self.decisions(), run_workflow=started.append)

        self.assertEqual(launched, ["release-v0.13"])
        self.assertEqual(started, ["release-v0.13"])

    def test_follow_waits_for_each_started_run_and_downloads_its_artifacts(self) -> None:
        """Ночь несёт артефакты запущенных прогонов сама: событие от токена страницы не будит."""
        decisions = self.decisions()
        self.module.dispatch(decisions, run_workflow=lambda line: None)
        downloaded = []

        outcomes = self.module.follow(
            decisions,
            Path("/tmp/large"),
            find=lambda line, since: 42,
            watch=lambda run_id: "success",
            download=lambda run_id, dest: downloaded.append((run_id, dest)),
        )

        self.assertEqual(outcomes, [{"line": "release-v0.13", "run_id": 42, "conclusion": "success"}])
        self.assertEqual(downloaded, [(42, Path("/tmp/large/release-v0.13"))])
        self.assertIn("прогон 42: success", next(d for d in decisions if d["line"] == "release-v0.13")["reason"])

    def test_find_started_run_takes_the_newest_run_created_after_dispatch(self) -> None:
        runs = [
            {"databaseId": 1, "createdAt": "2026-09-05T10:00:00Z"},
            {"databaseId": 2, "createdAt": "2026-09-05T10:26:00Z"},
            {"databaseId": 3, "createdAt": "2026-09-05T10:25:00Z"},
        ]

        found = self.module.find_started_run("main", "2026-09-05T10:24:00Z", list_runs=lambda line: runs, attempts=1, pause=0)

        self.assertEqual(found, 2)

    def test_nothing_moved_means_nothing_dispatched(self) -> None:
        memory = dict(self.memory)
        memory["https://site/data/release-v0.13/profiles/large.json"] = {"sha": self.heads["release-v0.13"], "at": "x"}

        self.assertEqual(self.module.dispatch(self.decisions(memory=memory), run_workflow=lambda line: None), [])


    def test_dispatch_prints_the_run_address_to_stderr_not_stdout(self) -> None:
        """stdout скрипта — строки для GITHUB_OUTPUT; адрес прогона от `gh` идёт в stderr."""
        import contextlib
        import io

        module = self.module
        calls = []

        class Completed:
            stdout = "https://github.com/x/actions/runs/1\n"

        original = module.subprocess.run
        module.subprocess.run = lambda command, **kwargs: (calls.append(command), Completed())[1]
        out, err = io.StringIO(), io.StringIO()
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                module.dispatch_large("main")
        finally:
            module.subprocess.run = original

        self.assertEqual(out.getvalue(), "")
        self.assertIn("runs/1", err.getvalue())
        self.assertEqual(calls[0][:6], ["gh", "workflow", "run", "unica-large.yml", "--ref", "main"])

if __name__ == "__main__":
    unittest.main()
