"""Допуск к тестам и проверка `PR metadata`: шаг workflow исполняется как есть над подставным `gh`."""

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

ROOT = Path(__file__).resolve().parents[2]
METADATA = ROOT / ".github/workflows/pr-metadata.yml"
GUARD = ROOT / ".github/workflows/issue-milestone-guard.yml"
CI = ROOT / ".github/workflows/ci.yml"
FAKE_GH = """#!/usr/bin/env bash
echo "$*" > "$FAKE_ARGS"
cat "$FAKE_RESPONSE"
"""
OWNER = "zeegin"


def events(actor):
    return {"nodes": [{"actor": {"login": actor}}] if actor else []}


def issue(number, milestone=True):
    return {"number": number, "milestone": {"number": 1} if milestone else None}


def response(actor=OWNER, assignees=1, issues=()) -> dict:
    return {"data": {"repository": {"pullRequest": {
        "milestone": {"number": 1} if actor else None,
        "assignees": {"totalCount": assignees},
        "timelineItems": events(actor),
        "closingIssuesReferences": {"nodes": list(issues)},
    }}}}


def load(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


class WorkflowShapeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.metadata = load(METADATA)
        self.ci = load(CI)
        self.job = self.metadata["jobs"]["metadata"]
        self.gate = self.ci["jobs"]["admission"]
        self.tests = self.ci["jobs"]["unica-ci"]

    def test_the_required_check_has_a_stable_name_and_no_condition(self) -> None:
        self.assertEqual(self.job["name"], "PR metadata")
        self.assertNotIn("if", self.job)
        self.assertNotIn("needs", self.job)

    def test_it_runs_on_every_metadata_change_and_in_the_queue_without_the_code_of_the_pr(self) -> None:
        on = self.metadata[True]
        self.assertEqual(set(on), {"pull_request", "merge_group"})
        self.assertEqual(
            set(on["pull_request"]["types"]),
            {"opened", "reopened", "ready_for_review", "edited", "milestoned", "demilestoned", "assigned", "unassigned", "synchronize"},
        )
        self.assertEqual(self.metadata["permissions"], {"contents": "read", "pull-requests": "read", "issues": "read"})
        for job in (self.job, self.gate):
            self.assertFalse(any("uses" in step for step in job["steps"]))

    def test_the_gate_and_the_check_apply_one_rule(self) -> None:
        check = self.job["steps"][0]
        gate = next(step for step in self.gate["steps"] if step["id"] == "admission")
        self.assertEqual(check["run"], gate["run"])
        self.assertEqual(check["env"]["MILESTONE_OWNER"], gate["env"]["MILESTONE_OWNER"])
        self.assertEqual(check["env"]["STRICT"], "true")
        self.assertEqual(gate["env"]["STRICT"], "false")

    def test_tests_wait_for_the_gate_and_the_queue_is_always_admitted(self) -> None:
        self.assertEqual(self.tests["needs"], "admission")
        self.assertEqual(self.tests["if"], "${{ needs.admission.outputs.admitted == 'true' }}")
        self.assertIn("github.event_name == 'merge_group' ||", self.gate["outputs"]["admitted"])
        self.assertEqual(self.ci["jobs"]["windows-clippy"]["if"], "github.event_name == 'merge_group'")

    def test_a_milestone_on_a_green_head_keeps_the_result_and_runs_nothing(self) -> None:
        kept = next(step for step in self.gate["steps"] if step["id"] == "kept")
        self.assertIn("github.event.action == 'milestoned'", kept["if"])
        self.assertIn("steps.admission.outputs.admitted == 'true'", kept["if"])
        self.assertIn('select(.status == "completed" and .conclusion != "skipped")', kept["run"])
        self.assertEqual(self.tests["env"]["KEPT"], "${{ needs.admission.outputs.kept }}")
        for step in self.tests["steps"]:
            with self.subTest(step=step.get("name") or step.get("uses") or step.get("run")):
                self.assertIn("env.KEPT != 'true'", step["if"])


class GuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = load(GUARD)
        self.job = self.workflow["jobs"]["guard"]

    def test_it_acts_only_on_a_foreign_milestone_with_the_least_rights_and_no_code(self) -> None:
        self.assertEqual(self.workflow[True], {"issues": {"types": ["milestoned"]}})
        self.assertEqual(self.workflow["permissions"], {"issues": "write"})
        self.assertEqual(self.job["if"], "${{ github.event.sender.login != 'zeegin' }}")
        self.assertFalse(any("uses" in step for step in self.job["steps"]))

    def test_the_tests_do_not_listen_to_issue_events(self) -> None:
        self.assertNotIn("issues", load(CI)[True])
        self.assertNotIn("issues", load(METADATA)[True])

    def test_it_removes_the_milestone_and_explains(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gh = root / "gh"
            gh.write_text('#!/usr/bin/env bash\necho "$*" >> calls.txt\n', encoding="utf-8", newline="\n")
            gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
            variables = {**os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}", "GITHUB_REPOSITORY": "o/r", "ISSUE_NUMBER": "5"}
            completed = subprocess.run(["bash", "-c", self.job["steps"][0]["run"].replace("\r\n", "\n")], cwd=root, env=variables,
                                       capture_output=True, text=True, encoding="utf-8")
            self.assertEqual(completed.returncode, 0, completed.stderr)
            calls = (root / "calls.txt").read_text(encoding="utf-8").splitlines()
        self.assertEqual(calls, ["issue edit 5 --repo o/r --remove-milestone",
                                 "issue comment 5 --repo o/r --body Веху ставит владелец после чтения issue"])


@unittest.skipUnless(shutil.which("jq"), "шаг читает ответ через jq; на раннере он есть")
class CheckStepTests(unittest.TestCase):
    def run_step(self, answer: dict, *, strict=True, **env: str) -> tuple[subprocess.CompletedProcess, str, str]:
        script = load(METADATA)["jobs"]["metadata"]["steps"][0]["run"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gh = root / "gh"
            gh.write_text(FAKE_GH, encoding="utf-8", newline="\n")
            gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
            (root / "response.json").write_text(json.dumps(answer), encoding="utf-8")
            variables = {
                **os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}", "GITHUB_REPOSITORY": "o/r",
                "GITHUB_REF": "refs/pull/7/merge", "PR_NUMBER": "7", "MILESTONE_OWNER": OWNER,
                "STRICT": "true" if strict else "false", "GITHUB_OUTPUT": "output.txt",
                "FAKE_RESPONSE": "response.json", "FAKE_ARGS": "args.txt", **env,
            }
            completed = subprocess.run(["bash", "-c", script.replace("\r\n", "\n")], cwd=root, env=variables,
                                       capture_output=True, text=True, encoding="utf-8")
            read = lambda name: (root / name).read_text(encoding="utf-8") if (root / name).exists() else ""
            return completed, read("args.txt"), read("output.txt")

    def assert_verdict(self, answer: dict, admitted: bool, *messages: str, passes: bool | None = None) -> None:
        completed, _, output = self.run_step(answer)
        self.assertIn(f"admitted={str(admitted).lower()}", output, completed.stdout + completed.stderr)
        self.assertEqual(completed.returncode == 0, admitted if passes is None else passes, completed.stdout + completed.stderr)
        for message in messages:
            self.assertIn(f"::error::{message}", completed.stdout)

    def test_a_pull_request_without_an_issue_needs_the_owners_milestone(self) -> None:
        self.assert_verdict(response(), True)
        self.assert_verdict(response(actor="agent"), False, "Веху на PR поставил agent, нужна от zeegin")
        self.assert_verdict(response(actor=None), False, "Поставьте веху: на PR без issue её ставит zeegin")

    def test_a_linked_issue_needs_any_milestone_the_guard_removes_foreign_ones(self) -> None:
        self.assert_verdict(response(actor="agent", issues=[issue(5)]), True)
        self.assert_verdict(response(actor=None, issues=[issue(5)]), False, "Поставьте веху: скопируйте её с issue")
        self.assert_verdict(response(actor="agent", issues=[issue(5), issue(8, False)]), False,
                            "Issue #8 без вехи, PR не рассматривается")

    def test_a_missing_assignee_reddens_the_check_but_does_not_stop_the_tests(self) -> None:
        self.assert_verdict(response(assignees=0), True, "Укажите ответственного", passes=False)
        completed, _, output = self.run_step(response(assignees=0), strict=False)
        self.assertEqual(completed.returncode, 0)
        self.assertIn("admitted=true", output)
        self.assertNotIn("ответственного", completed.stdout)

    def test_the_gate_never_fails_it_only_withholds_the_tests(self) -> None:
        completed, _, output = self.run_step(response(actor=None), strict=False)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("admitted=false", output)
        self.assertIn("::notice::", completed.stdout)
        self.assertNotIn("::error::", completed.stdout)

    def test_the_queue_takes_the_number_from_the_ref_and_checks_again(self) -> None:
        ref = "refs/heads/gh-readonly-queue/main/pr-2494-b8d576dca6e9ffa40b9b10fe006472736fcc5a23"
        completed, arguments, _ = self.run_step(response(actor=None), GITHUB_REF=ref, PR_NUMBER="")
        self.assertEqual(completed.returncode, 1)
        self.assertIn("number=2494", arguments)

    def test_an_unknown_ref_is_refused_so_nothing_passes_unchecked(self) -> None:
        completed, arguments, _ = self.run_step(response(), GITHUB_REF="refs/heads/main", PR_NUMBER="")
        self.assertEqual(completed.returncode, 1)
        self.assertEqual(arguments, "")
        self.assertIn("Номер PR не определён", completed.stdout)


if __name__ == "__main__":
    unittest.main()
