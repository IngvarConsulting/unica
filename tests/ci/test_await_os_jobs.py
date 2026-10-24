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

    def test_a_step_of_a_job_reports_its_own_outcome(self):
        # The pull request job lints and runs Python: a red lint must not
        # pass for a red Python, and the other way round.
        checks = {**job("Lint and Python", conclusion="failure"), "runner_name": "GitHub Actions 7",
                  "steps": [{"name": "Python", "conclusion": "success"}, {"name": "Clippy", "conclusion": "failure"}]}
        result = self.wait([[checks]], expected=("Lint and Python::Python", "Lint and Python::Clippy", "Lint and Python::Absent"))
        # A step the list never names waits until the deadline and then takes
        # the job's conclusion (#2026): a red job never passes for it.
        self.assertEqual(result, {"Lint and Python::Python": "success", "Lint and Python::Clippy": "failure",
                                  "Lint and Python::Absent": "failure"})
        never_started = {**job("Lint and Python", conclusion="cancelled"), "runner_name": "", "steps": []}
        self.assertEqual(self.wait([[never_started]], expected=("Lint and Python::Python",)),
                         {"Lint and Python::Python": "not_acquired"})

    def test_an_unwritten_outcome_of_a_completed_job_is_still_pending(self):
        # Jobs API marks a job completed before it fills the conclusions in
        # (#2008, seen on PR #1610: Clippy succeeded, the gate read "unknown"
        # and went red).
        def checks(step_conclusion, job_conclusion="success"):
            return {**job("Lint and Python", conclusion=job_conclusion), "runner_name": "GitHub Actions 7",
                    "steps": [{"name": "Clippy", "conclusion": step_conclusion}]}
        expected = ("Lint and Python::Clippy",)
        self.assertEqual(self.wait([[checks(None)], [checks("")], [checks("success")]], expected=expected),
                         {"Lint and Python::Clippy": "success"})
        self.assertEqual(self.wait([[checks(None, None)], [checks("success")]], expected=expected),
                         {"Lint and Python::Clippy": "success"})
        self.assertEqual(self.wait([[job(MAC, conclusion=None)], [job(MAC)]]), {MAC: "success"})
        # A step never written by the deadline takes the job's conclusion
        # (#2026); a job never written is out of time.
        self.assertEqual(self.wait([[checks(None)]], expected=expected, timeout=60),
                         {"Lint and Python::Clippy": "success"})
        self.assertEqual(self.wait([[checks(None, "failure")]], expected=expected, timeout=60),
                         {"Lint and Python::Clippy": "failure"})
        self.assertEqual(self.wait([[job(MAC, conclusion=None)]], timeout=60), {MAC: "timed_out"})

    def test_a_completed_job_without_steps_answers_for_its_steps(self):
        # PR #2012: Jobs API kept the completed "Lint and Python" job with an
        # empty step list; the job's own conclusion stands for each step.
        def checks(job_conclusion):
            return {**job("Lint and Python", conclusion=job_conclusion), "runner_name": "GitHub Actions 7", "steps": []}
        expected = ("Lint and Python::Python", "Lint and Python::Clippy")
        self.assertEqual(self.wait([[checks("success")]], expected=expected),
                         {"Lint and Python::Python": "success", "Lint and Python::Clippy": "success"})
        self.assertEqual(self.wait([[checks("failure")]], expected=expected),
                         {"Lint and Python::Python": "failure", "Lint and Python::Clippy": "failure"})

    def test_timeout_keeps_the_outcome_of_steps_that_did_finish(self):
        checks = {**job("Lint and Python"), "runner_name": "GitHub Actions 7",
                  "steps": [{"name": "Python", "conclusion": "failure"}, {"name": "Clippy", "conclusion": None}]}
        result = self.wait([[checks]], expected=("Lint and Python::Python", "Lint and Python::Clippy"), timeout=60)
        self.assertEqual(result, {"Lint and Python::Python": "failure", "Lint and Python::Clippy": "success"})

    def test_a_partial_step_list_waits_for_its_steps_then_takes_the_job_conclusion(self):
        # #2026, PR #2016: the completed job listed Clippy without a conclusion
        # and no Python step at all; the gate read both as missing.
        expected = ("Lint and Python::Python", "Lint and Python::Clippy")
        def checks(job_conclusion, steps):
            return {**job("Lint and Python", conclusion=job_conclusion), "runner_name": "GitHub Actions 7",
                    "steps": steps}
        partial = [{"name": "Set up job", "conclusion": "success"}, {"name": "Clippy", "conclusion": None},
                   {"name": "Post Run actions/checkout", "conclusion": None}]
        written = [{"name": "Python", "conclusion": "success"}, {"name": "Clippy", "conclusion": "failure"}]
        # Written later: the written outcomes stand.
        self.assertEqual(self.wait([[checks("failure", partial)], [checks("failure", written)]], expected=expected),
                         {"Lint and Python::Python": "success", "Lint and Python::Clippy": "failure"})
        # Never written: the job's conclusion, never a success of a red job.
        for job_conclusion in ("success", "failure"):
            self.assertEqual(self.wait([[checks(job_conclusion, partial)]], expected=expected, timeout=60),
                             {name: job_conclusion for name in expected})

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
