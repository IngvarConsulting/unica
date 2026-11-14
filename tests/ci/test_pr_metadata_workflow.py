"""Допуск к тестам и проверка `PR metadata`: шаг workflow исполняется как есть над подставным `gh`."""

from __future__ import annotations

import json
import os
import shutil
import stat
import subprocess
import sys
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


def response(actor=OWNER, assignees=1, issues=(), draft=False) -> dict:
    return {"data": {"repository": {"pullRequest": {
        "isDraft": draft,
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
        # Файл берётся из main: PR не может поправить его и получить зелёную проверку.
        # Кроме перезапуска по отзыву: тот прогон берёт файл из ветки PR.
        self.assertEqual(set(on), {"pull_request_target", "pull_request_review", "merge_group"})
        self.assertEqual(set(on["pull_request_review"]["types"]), {"submitted", "dismissed"})
        self.assertEqual(
            set(on["pull_request_target"]["types"]),
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

    def test_the_gate_asks_github_whether_the_pr_is_a_draft(self) -> None:
        # Сохранённое событие прогона знает черновиковость на момент события;
        # выживший прогон и его перезапуск должны спрашивать, что истинно сейчас.
        gate = next(step for step in self.gate["steps"] if step["id"] == "admission")
        self.assertEqual(gate["if"], "${{ github.event_name == 'pull_request' }}")
        self.assertNotIn("draft", gate["if"])
        self.assertIn("isDraft", gate["run"])

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

    def test_a_draft_withholds_the_tests_without_reddening_the_check(self) -> None:
        for strict in (False, True):
            with self.subTest(strict=strict):
                completed, _, output = self.run_step(response(draft=True), strict=strict)
                self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
                self.assertIn("admitted=false", output)
                self.assertIn("::notice::PR #7 — черновик", completed.stdout)
                self.assertNotIn("::error::", completed.stdout)
        completed, _, output = self.run_step(response(draft=False), strict=False)
        self.assertIn("admitted=true", output)

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


OWNER_GH = """#!{python}
import json, os, sys
args = " ".join(sys.argv[1:])
fixture = json.load(open(os.environ["FAKE_FIXTURE"], encoding="utf-8"))
with open(os.environ["FAKE_ARGS"], "a", encoding="utf-8") as calls:
    calls.write(args + "\\n")
if "/contents/.github/CODEOWNERS" in args:
    print(fixture["codeowners"], end="")
elif "/files" in args:
    for name in fixture["files"]:
        print(name)
elif "/reviews" in args:
    for login, state, commit in fixture["reviews"]:
        print(login + "\\t" + state + "\\t" + commit)
else:
    print(fixture["author"] + " " + fixture["head"])
"""
CODEOWNERS = "# comment\n/arch/rules/** @zeegin\n/.github/** @zeegin\n/docs/release-runbook.md @zeegin\n"
HEAD = "a" * 40


class OwnerReviewTests(unittest.TestCase):
    """PR не от владельца, который меняет путь из CODEOWNERS, ждёт его одобрения на текущем head."""

    def setUp(self) -> None:
        self.step = next(step for step in load(METADATA)["jobs"]["metadata"]["steps"] if step["name"] == "Одобрение владельца")

    def run_check(self, author: str, files, reviews=(), **variables):
        bash = shutil.which("bash")
        if bash is None:
            self.skipTest("bash is not available")
        # The step calls `python3`; this interpreter stands in for it on every host.
        python = Path(sys.executable).as_posix()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gh = root / "gh"
            gh.write_text(OWNER_GH.format(python=python), encoding="utf-8", newline="\n")
            gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
            shim = root / "python3"
            shim.write_text(f'#!/usr/bin/env bash\nexec "{python}" "$@"\n', encoding="utf-8", newline="\n")
            shim.chmod(0o755)
            fixture = root / "fixture.json"
            fixture.write_text(json.dumps({"author": author, "head": HEAD, "codeowners": CODEOWNERS,
                                           "files": list(files), "reviews": [list(review) for review in reviews]}), encoding="utf-8")
            environment = {**os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}", "GITHUB_REPOSITORY": "o/r",
                           "PR_NUMBER": "7", "CODE_OWNER": self.step["env"]["CODE_OWNER"], "FAKE_FIXTURE": str(fixture),
                           "FAKE_ARGS": str(root / "calls"), **variables}
            completed = subprocess.run([bash, "-c", self.step["run"].replace("\r\n", "\n")], cwd=root, env=environment,
                                       capture_output=True, text=True, encoding="utf-8")
            calls = (root / "calls").read_text(encoding="utf-8") if (root / "calls").exists() else ""
        return completed, calls

    def test_the_step_runs_after_a_refused_milestone_and_reads_owners_from_main(self) -> None:
        self.assertEqual(self.step["if"], "${{ !cancelled() }}")
        self.assertEqual(self.step["env"]["CODE_OWNER"], OWNER)
        self.assertIn("CODEOWNERS?ref=main", self.step["run"])

    def test_a_pr_by_the_owner_passes_without_asking_for_files_or_reviews(self) -> None:
        completed, calls = self.run_check(OWNER, [".github/workflows/ci.yml"])
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertNotIn("/files", calls)

    def test_a_pr_that_touches_no_owned_path_passes_without_a_review(self) -> None:
        completed, calls = self.run_check("pasha", ["crates/unica-coder/src/lib.rs", "docs/other.md"])
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertNotIn("/reviews", calls)

    def test_an_owned_path_needs_the_owners_approval_of_the_current_head(self) -> None:
        for path in (".github/workflows/ci.yml", "docs/release-runbook.md", "arch/rules/core/x.md"):
            with self.subTest(path=path):
                completed, _ = self.run_check("pasha", ["src/a.rs", path])
                self.assertEqual(completed.returncode, 1)
                self.assertIn("нужен одобряющий отзыв zeegin", completed.stdout)
                self.assertIn(path, completed.stdout)
                completed, _ = self.run_check("pasha", [path], [(OWNER, "APPROVED", HEAD)])
                self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)

    def test_an_approval_of_an_older_head_by_someone_else_or_withdrawn_does_not_count(self) -> None:
        for reviews in ([(OWNER, "APPROVED", "b" * 40)], [("viktor", "APPROVED", HEAD)],
                        [(OWNER, "APPROVED", HEAD), (OWNER, "CHANGES_REQUESTED", HEAD)],
                        [(OWNER, "APPROVED", HEAD), (OWNER, "DISMISSED", HEAD)]):
            with self.subTest(reviews=reviews):
                completed, _ = self.run_check("pasha", [".github/CODEOWNERS"], reviews)
                self.assertEqual(completed.returncode, 1)

    def test_a_comment_after_the_approval_keeps_it(self) -> None:
        completed, _ = self.run_check("pasha", [".github/CODEOWNERS"], [(OWNER, "APPROVED", HEAD), (OWNER, "COMMENTED", HEAD)])
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)

    def test_the_queue_checks_again_with_the_number_from_its_ref(self) -> None:
        ref = "refs/heads/gh-readonly-queue/main/pr-2494-b8d576dca6e9ffa40b9b10fe006472736fcc5a23"
        completed, calls = self.run_check("pasha", [".github/CODEOWNERS"], GITHUB_REF=ref, PR_NUMBER="")
        self.assertEqual(completed.returncode, 1)
        self.assertIn("pulls/2494", calls)

    def test_the_runbook_is_owned(self) -> None:
        self.assertIn("/docs/release-runbook.md @zeegin", (ROOT / ".github/CODEOWNERS").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
