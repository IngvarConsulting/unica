"""Финальная версия из кандидата: доказательство выпуска, версия, служебный коммит и тег ключом."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/release-from-rc.yml"
COMMIT = "d" * 40
RC = "v0.15.0-rc.2"


def load():
    spec = importlib.util.spec_from_file_location("release_final", ROOT / "scripts/ci/release-final.py")
    module = importlib.util.module_from_spec(spec)
    # `dataclass` ищет модуль класса в `sys.modules`.
    sys.modules["release_final"] = module
    spec.loader.exec_module(module)
    return module


class Fakes:
    def __init__(self, module, *, tags=("v0.15.0-rc.1", RC, "v0.14.2"), draft=False, conclusion="success",
                 run_branch=RC, run_sha=COMMIT, in_marketplace=True, marketplace_error="404 Not Found",
                 line_exists=False, line_error="404 Not Found"):
        self.module = module
        self.tags, self.draft, self.conclusion = tags, draft, conclusion
        self.run_branch, self.run_sha, self.in_marketplace = run_branch, run_sha, in_marketplace
        self.marketplace_error = marketplace_error
        self.line_exists, self.line_error = line_exists, line_error
        self.commands = []

    def gh(self, path, **_):
        if "matching-refs/tags/v" in path:
            return [{"ref": f"refs/tags/{tag}"} for tag in self.tags]
        if path.startswith("repos/o/r/releases/tags/"):
            return {"draft": self.draft}
        if "/workflows/release.yml/runs" in path:
            self.release_runs_path = path
            return {"workflow_runs": [{"conclusion": self.conclusion, "head_branch": self.run_branch,
                                       "head_sha": self.run_sha, "created_at": "2026-11-09",
                                       "html_url": "https://run.invalid/7"}]}
        if path.startswith("repos/IngvarConsulting/unica-marketplace/git/ref/tags/"):
            if not self.in_marketplace:
                raise self.module.alpha.AlphaError(f"gh api GET ...: {self.marketplace_error}")
            return {"ref": path.rsplit("/git/", 1)[1]}
        if path.startswith("repos/o/r/branches/"):
            self.branch_path = path
            if self.line_exists:
                return {"name": path.rsplit("/", 1)[1]}
            raise self.module.alpha.AlphaError(f"gh api GET ...: {self.line_error}")
        raise AssertionError(path)

    def runner(self, command, cwd=None, env=None):
        self.commands.append((command, env))
        if command[:2] == ["git", "rev-parse"]:
            return COMMIT + "\n"
        return ""


class ProofTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def prove(self, fakes, tag=RC):
        return self.module.prove("o/r", tag, fakes.gh, lambda arguments: fakes.runner(["git", *arguments]))

    def test_a_green_release_of_the_newest_candidate_published_in_next_is_the_proof(self) -> None:
        fakes = Fakes(self.module)
        proof = self.prove(fakes)
        self.assertEqual((proof.rc, proof.commit, proof.run_url), (RC, COMMIT, "https://run.invalid/7"))
        self.assertIn("event=push", fakes.release_runs_path)
        self.assertIn(f"head_sha={COMMIT}", fakes.release_runs_path)

    def test_every_missing_piece_is_a_refusal(self) -> None:
        cases = {
            "нет": Fakes(self.module, tags=("v0.15.0-rc.1",)),
            "уже вышел": Fakes(self.module, tags=(RC, "v0.15.0")),
            "более поздний": Fakes(self.module, tags=(RC, "v0.15.0-rc.3")),
            "не опубликован": Fakes(self.module, draft=True),
            "зелёного выпуска": Fakes(self.module, conclusion="failure"),
            "зелёного выпуска по тегу": Fakes(self.module, run_branch="main"),
            "зелёного выпуска по": Fakes(self.module, run_sha="e" * 40),
            "канале next": Fakes(self.module, in_marketplace=False),
            "release-v0.15 уже есть": Fakes(self.module, line_exists=True),
            "release-v0.15 не прочитана": Fakes(self.module, line_error="502 Bad Gateway"),
        }
        for reason, fakes in cases.items():
            with self.subTest(reason=reason), self.assertRaisesRegex(self.module.FinalError, reason):
                self.prove(fakes)

    def test_an_unreadable_marketplace_is_not_reported_as_an_unpublished_candidate(self) -> None:
        with self.assertRaisesRegex(self.module.FinalError, "не прочитан: .*rate limit"):
            self.prove(Fakes(self.module, in_marketplace=False, marketplace_error="403 API rate limit exceeded"))

    def test_a_later_candidate_of_another_core_does_not_block(self) -> None:
        proof = self.prove(Fakes(self.module, tags=(RC, "v0.16.0-rc.5", "v0.14.9-rc.7")))
        self.assertEqual(proof.rc, RC)

    def test_a_tag_that_is_not_a_candidate_is_refused(self) -> None:
        for tag in ("v0.15.0", "v0.15.0-alpha.4", "0.15.0-rc.2"):
            with self.subTest(tag=tag), self.assertRaisesRegex(self.module.FinalError, "не тег кандидата"):
                self.prove(Fakes(self.module), tag)


class PublishTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def test_the_version_commit_is_cut_on_the_candidate_and_the_tag_is_pushed_with_the_key(self) -> None:
        fakes = Fakes(self.module)
        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.dict(os.environ, {"RELEASE_TAG_KEY": "-----KEY-----", "GITHUB_OUTPUT": str(Path(directory) / "out")}):
            tag = self.module.publish(Path(directory), "o/r", RC, dry_run=False, gh=fakes.gh, runner=fakes.runner)
            self.assertEqual((Path(directory) / "out").read_text(encoding="utf-8"), "tag=v0.15.0\n")
        self.assertEqual(tag, "v0.15.0")
        commands = [command for command, _ in fakes.commands]
        checkout = commands.index(["git", "checkout", "--detach", COMMIT])
        bump = commands.index([sys.executable, "scripts/dev/bump-version.py", "0.15.0"])
        push = next(index for index, command in enumerate(commands) if command[:2] == ["git", "push"])
        self.assertLess(checkout, bump)
        self.assertLess(bump, push)
        # Тег и ветка линии `release-v0.15` уходят одним атомарным push.
        self.assertEqual(commands[push], ["git", "push", "--atomic", "git@github.com:o/r.git", "refs/tags/v0.15.0",
                                          "HEAD:refs/heads/release-v0.15"])
        self.assertEqual(fakes.branch_path, "repos/o/r/branches/release-v0.15")
        self.assertIn("StrictHostKeyChecking=yes", fakes.commands[push][1]["GIT_SSH_COMMAND"])

    def test_a_failed_version_check_stops_before_the_commit_and_the_push(self) -> None:
        fakes = Fakes(self.module)
        original = fakes.runner

        def runner(command, cwd=None, env=None):
            if "scripts/ci/check-version-contract.py" in command:
                fakes.commands.append((command, env))
                raise self.module.alpha.AlphaError("version contract drift")
            return original(command, cwd, env)

        with self.assertRaises(self.module.alpha.AlphaError):
            self.module.publish(Path("."), "o/r", RC, dry_run=False, gh=fakes.gh, runner=runner)
        self.assertFalse(any(command[0] == "git" and {"commit", "tag", "push"} & set(command)
                             for command, _ in fakes.commands))

    def test_a_dry_run_proves_and_names_the_version_without_touching_anything(self) -> None:
        fakes = Fakes(self.module)
        with tempfile.TemporaryDirectory() as directory,              mock.patch.dict(os.environ, {"GITHUB_OUTPUT": str(Path(directory) / "out")}):
            self.assertEqual(self.module.publish(Path("."), "o/r", RC, dry_run=True, gh=fakes.gh, runner=fakes.runner),
                             "v0.15.0")
            self.assertFalse((Path(directory) / "out").exists())
        self.assertFalse(any(command[0] == "git" and {"commit", "tag", "push", "checkout"} & set(command)
                             for command, _ in fakes.commands))

    def test_without_the_key_nothing_is_pushed(self) -> None:
        fakes = Fakes(self.module)
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {"RELEASE_TAG_KEY": ""}), \
             self.assertRaisesRegex(self.module.FinalError, "RELEASE_TAG_KEY"):
            self.module.publish(Path(directory), "o/r", RC, dry_run=False, gh=fakes.gh, runner=fakes.runner)
        self.assertFalse(any(command[:2] == ["git", "push"] for command, _ in fakes.commands))


class WorkflowTests(unittest.TestCase):
    def test_only_an_administrator_on_main_reaches_the_job_with_the_tag_key(self) -> None:
        workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        self.assertEqual(set(workflow[True]), {"workflow_dispatch"})
        self.assertTrue(workflow[True]["workflow_dispatch"]["inputs"]["rc"]["required"])
        self.assertEqual(workflow["permissions"], {"contents": "read", "actions": "read"})
        command = 'release-rc.py admit --actor "$ACTOR" --ref "$GITHUB_REF"'
        jobs = workflow["jobs"]
        self.assertNotIn("environment", jobs["admit"])
        self.assertNotIn("secrets.", json.dumps(jobs["admit"]))
        self.assertIn(command, jobs["admit"]["steps"][-1]["run"])
        final = jobs["final"]
        self.assertEqual((final["needs"], final["environment"]), ("admit", "alpha"))
        steps = final["steps"]
        recheck = next(index for index, step in enumerate(steps) if step.get("id") == "admit")
        publish = next(index for index, step in enumerate(steps) if step.get("id") == "publish")
        self.assertLess(recheck, publish)
        self.assertIn(command, steps[recheck]["run"])
        self.assertNotIn("RELEASE_TAG_KEY", json.dumps(steps[:publish]))
        self.assertEqual(steps[publish]["env"]["RELEASE_TAG_KEY"], "${{ secrets.RELEASE_TAG_KEY }}")
        self.assertIn('release-final.py publish --rc "$RC_TAG"', steps[publish]["run"])
        checkout = steps[0]
        self.assertEqual((checkout["with"]["ref"], checkout["with"]["fetch-depth"]), ("main", 0))

    def test_every_release_button_shares_one_queue(self) -> None:
        groups = {
            name: yaml.safe_load((ROOT / ".github/workflows" / name).read_text(encoding="utf-8"))["concurrency"]
            for name in ("rc-from-alpha.yml", "release-from-rc.yml", "release-from-line.yml")
        }
        self.assertEqual(groups["rc-from-alpha.yml"], groups["release-from-rc.yml"])
        self.assertEqual(groups["rc-from-alpha.yml"], groups["release-from-line.yml"])
        self.assertFalse(groups["release-from-rc.yml"]["cancel-in-progress"])

    def test_the_tag_key_is_read_only_in_the_alpha_environment(self) -> None:
        holders = []
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            workflow = yaml.safe_load(path.read_text(encoding="utf-8"))
            for name, job in (workflow.get("jobs") or {}).items():
                if "secrets.RELEASE_TAG_KEY" in json.dumps(job) and "uses" not in job:
                    holders.append((path.name, name, job.get("environment")))
        self.assertEqual(
            holders,
            [("rc-from-alpha.yml", "rc", "alpha"), ("release-from-line.yml", "patch", "alpha"),
             ("release-from-rc.yml", "final", "alpha"), ("release.yml", "alpha-tag", "alpha")],
        )


if __name__ == "__main__":
    unittest.main()
