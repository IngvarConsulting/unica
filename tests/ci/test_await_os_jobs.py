"""`Unica CI` ждёт джобы других ОС по jobs API: исход, отсутствие и срок."""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import await_os_jobs as waiting  # noqa: E402


MAC = "Rust (macos-14)"
WINDOWS = "Rust (windows-latest)"


def job(name, status="completed", conclusion="success"):
    return {"name": name, "status": status, "conclusion": conclusion if status == "completed" else None}


class Clock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class AwaitOsJobsTests(unittest.TestCase):
    def wait(self, responses, expected=(MAC,), timeout=60):
        clock = Clock()
        replies = list(responses)

        def read():
            # The last reply repeats: a broken API stays broken.
            reply = replies.pop(0) if len(replies) > 1 else replies[0]
            if isinstance(reply, Exception):
                raise reply
            return reply

        return waiting.wait(list(expected), read, timeout=timeout, interval=20, clock=clock, sleep=clock.sleep)

    def test_completed_job_reports_its_conclusion_after_it_finishes(self):
        result = self.wait([
            [job("Unica CI", "in_progress"), job(MAC, "queued")],
            [job("Unica CI", "in_progress"), job(MAC, "in_progress")],
            [job("Unica CI", "in_progress"), job(MAC, conclusion="failure")],
        ])
        self.assertEqual(result, {MAC: "failure"})

    def test_a_job_cancelled_without_a_runner_was_not_acquired(self):
        never_started = {**job(MAC, conclusion="cancelled"), "runner_name": ""}
        cancelled = {**job(MAC, conclusion="cancelled"), "runner_name": "GitHub Actions 1000"}
        self.assertEqual(self.wait([[never_started]]), {MAC: "not_acquired"})
        self.assertEqual(self.wait([[cancelled]]), {MAC: "cancelled"})

    def test_an_absent_job_is_missing_only_once_nothing_else_runs(self):
        # The matrix of the other runner may not be expanded yet.
        result = self.wait([
            [job("Unica CI", "in_progress"), job("Rust (${{ matrix.runner }})", "queued")],
            [job("Unica CI", "in_progress"), job("Rust (${{ matrix.runner }})", conclusion="skipped")],
        ])
        self.assertEqual(result, {MAC: "missing"})

    def test_timeout_keeps_the_outcome_of_jobs_that_did_finish(self):
        result = self.wait([[job("Unica CI", "in_progress"), job(MAC), job(WINDOWS, "in_progress")]],
                           expected=(MAC, WINDOWS), timeout=60)
        self.assertEqual(result, {MAC: "success", WINDOWS: "timed_out"})

    def test_api_errors_are_retried_and_never_read_as_success(self):
        result = self.wait([subprocess.CalledProcessError(1, ["gh"]), [job(MAC)]])
        self.assertEqual(result, {MAC: "success"})
        self.assertEqual(self.wait([subprocess.CalledProcessError(1, ["gh"])], timeout=40), {MAC: "timed_out"})

    def test_job_names_are_those_the_verifier_and_the_gate_expect(self):
        with tempfile.TemporaryDirectory() as temporary, \
             patch.dict(os.environ, {"GITHUB_REPOSITORY": "o/r", "GITHUB_RUN_ID": "1"}), \
             patch.object(waiting, "list_jobs", return_value=[job(MAC)]) as listed, \
             patch.object(sys, "argv", ["await_os_jobs.py", "--runners", '["ubuntu-latest", "macos-14"]',
                                        "--out", str(Path(temporary) / "jobs.json")]):
            self.assertEqual(waiting.main(), 0)
            self.assertEqual(json.loads((Path(temporary) / "jobs.json").read_text()), {MAC: "success"})
            listed.assert_called_with("o/r", "1")
        # Linux alone has nothing to wait for.
        with tempfile.TemporaryDirectory() as temporary, \
             patch.dict(os.environ, {"GITHUB_REPOSITORY": "o/r", "GITHUB_RUN_ID": "1"}), \
             patch.object(waiting, "list_jobs", side_effect=AssertionError("no API call")), \
             patch.object(sys, "argv", ["await_os_jobs.py", "--runners", '["ubuntu-latest"]',
                                        "--out", str(Path(temporary) / "jobs.json")]):
            self.assertEqual(waiting.main(), 0)
            self.assertEqual(json.loads((Path(temporary) / "jobs.json").read_text()), {})


if __name__ == "__main__":
    unittest.main()
