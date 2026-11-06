"""Проверка `PR metadata`: шаг workflow исполняется как есть над подставным `gh`."""

from __future__ import annotations

import json
import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

WORKFLOW = Path(__file__).resolve().parents[2] / ".github/workflows/pr-metadata.yml"
FAKE_GH = """#!/usr/bin/env bash
echo "$*" > "$FAKE_ARGS"
cat "$FAKE_RESPONSE"
"""


def response(milestone=True, assignees=1, issues=()) -> dict:
    return {"data": {"repository": {"pullRequest": {
        "milestone": {"number": 1} if milestone else None,
        "assignees": {"totalCount": assignees},
        "closingIssuesReferences": {"nodes": [
            {"number": number, "milestone": {"number": 1} if has else None} for number, has in issues
        ]},
    }}}}


class WorkflowShapeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        self.job = self.workflow["jobs"]["metadata"]

    def test_the_check_has_a_stable_name_and_no_condition(self) -> None:
        self.assertEqual(self.job["name"], "PR metadata")
        self.assertNotIn("if", self.job)
        self.assertNotIn("needs", self.job)

    def test_it_runs_on_every_metadata_change_and_in_the_queue_without_the_code_of_the_pr(self) -> None:
        on = self.workflow[True]
        self.assertEqual(set(on), {"pull_request", "merge_group"})
        self.assertEqual(
            set(on["pull_request"]["types"]),
            {"opened", "reopened", "ready_for_review", "edited", "milestoned", "demilestoned", "assigned", "unassigned", "synchronize"},
        )
        self.assertEqual(self.workflow["permissions"], {"contents": "read", "pull-requests": "read", "issues": "read"})
        self.assertFalse(any("uses" in step for step in self.job["steps"]))


@unittest.skipUnless(shutil.which("jq"), "шаг читает ответ через jq; на раннере он есть")
class CheckStepTests(unittest.TestCase):
    def run_step(self, answer: dict, **env: str) -> tuple[subprocess.CompletedProcess, str]:
        script = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]["metadata"]["steps"][0]["run"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gh = root / "gh"
            gh.write_text(FAKE_GH, encoding="utf-8", newline="\n")
            gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
            (root / "response.json").write_text(json.dumps(answer), encoding="utf-8")
            variables = {
                **os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}", "GITHUB_REPOSITORY": "o/r",
                "GITHUB_REF": "refs/pull/7/merge", "PR_NUMBER": "7",
                "FAKE_RESPONSE": str(root / "response.json"), "FAKE_ARGS": str(root / "args.txt"), **env,
            }
            completed = subprocess.run(["bash", "-c", script], env=variables, capture_output=True, text=True, encoding="utf-8")
            arguments = (root / "args.txt").read_text(encoding="utf-8") if (root / "args.txt").exists() else ""
        return completed, arguments

    def test_a_complete_pull_request_passes(self) -> None:
        completed, arguments = self.run_step(response(issues=[(5, True)]))
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertIn("number=7", arguments)

    def test_each_gap_has_its_own_message_and_all_are_named(self) -> None:
        completed, _ = self.run_step(response(milestone=False, assignees=0, issues=[(5, False), (6, True), (8, False)]))
        self.assertEqual(completed.returncode, 1)
        for message in ("Поставьте веху", "Укажите ответственного", "Issue #5 без вехи, PR не рассматривается",
                        "Issue #8 без вехи, PR не рассматривается"):
            self.assertIn(f"::error::{message}", completed.stdout)
        self.assertNotIn("Issue #6", completed.stdout)

    def test_the_queue_takes_the_number_from_the_ref_and_checks_again(self) -> None:
        ref = "refs/heads/gh-readonly-queue/main/pr-2494-b8d576dca6e9ffa40b9b10fe006472736fcc5a23"
        completed, arguments = self.run_step(response(milestone=False), GITHUB_REF=ref, PR_NUMBER="")
        self.assertEqual(completed.returncode, 1)
        self.assertIn("number=2494", arguments)
        self.assertIn("::error::Поставьте веху", completed.stdout)

    def test_an_unknown_ref_is_refused_so_nothing_passes_unchecked(self) -> None:
        completed, arguments = self.run_step(response(), GITHUB_REF="refs/heads/main", PR_NUMBER="")
        self.assertEqual(completed.returncode, 1)
        self.assertEqual(arguments, "")
        self.assertIn("Номер PR не определён", completed.stdout)


if __name__ == "__main__":
    unittest.main()
