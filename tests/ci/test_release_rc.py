"""Кандидат из проверенной alpha: доказательство, номер, служебный коммит и тег ключом."""

from __future__ import annotations

import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/rc-from-alpha.yml"
SLICE = "c" * 40
ALPHA = "v0.15.0-alpha.4"


def load():
    spec = importlib.util.spec_from_file_location("release_rc", ROOT / "scripts/ci/release-rc.py")
    module = importlib.util.module_from_spec(spec)
    # `dataclass` ищет модуль класса в `sys.modules`.
    sys.modules["release_rc"] = module
    spec.loader.exec_module(module)
    return module


class Fakes:
    def __init__(self, module, *, conclusion="success", alpha_job="success", in_main=True, body_slice=SLICE,
                 checked=SLICE, trees=None, tags=("v0.15.0-rc.1", "v0.14.2")):
        self.module = module
        self.conclusion, self.alpha_job, self.in_main = conclusion, alpha_job, in_main
        self.body_slice, self.checked, self.tags = body_slice, checked, tags
        self.trees = trees or {}
        self.commands = []

    def gh(self, path, **_):
        if path.startswith("repos/o/r/releases/tags/"):
            return {"draft": False, "body": f"Срез main@{self.body_slice}."}
        if "/workflows/nightly.yml/runs" in path:
            return {"workflow_runs": [{"id": 9, "conclusion": self.conclusion, "head_sha": self.checked,
                                       "created_at": "2026-11-08", "html_url": "https://run.invalid/9"}]}
        if path.startswith("repos/o/r/actions/runs/9/jobs"):
            return {"jobs": [{"name": "Полный прогон / Alpha — служебный коммит и тег", "conclusion": self.alpha_job}]}
        if "matching-refs/tags/v" in path:
            return [{"ref": f"refs/tags/{tag}"} for tag in self.tags]
        raise AssertionError(path)

    def runner(self, command, cwd=None, env=None):
        self.commands.append((command, env))
        if command[:2] == ["git", "rev-parse"]:
            target = command[2]
            if target.startswith(ALPHA):
                return SLICE + "\n"
            return self.trees.get(target, "t" * 40) + "\n"
        if command[:3] == ["git", "merge-base", "--is-ancestor"] and not self.in_main:
            raise self.module.alpha.AlphaError("not an ancestor")
        return ""


class NumberTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def test_the_next_number_follows_the_candidates_of_the_same_core(self) -> None:
        self.assertEqual(self.module.next_rc("0.15.0", ["v0.15.0-rc.1", "v0.15.0-rc.10", "v0.14.0-rc.30"]), "v0.15.0-rc.11")
        self.assertEqual(self.module.next_rc("0.16.0", ["v0.15.0-rc.1"]), "v0.16.0-rc.1")

    def test_a_released_core_gets_no_candidate(self) -> None:
        with self.assertRaisesRegex(self.module.RcError, "v0.15.0 уже вышел"):
            self.module.next_rc("0.15.0", ["v0.15.0", "v0.15.0-rc.2"])


class ProofTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def prove(self, fakes: Fakes):
        return self.module.prove("o/r", ALPHA, fakes.gh, lambda arguments: fakes.runner(["git", *arguments]))

    def test_a_green_night_that_set_the_alpha_on_its_slice_is_the_proof(self) -> None:
        proof = self.prove(Fakes(self.module))
        self.assertEqual((proof.slice, proof.run_url), (SLICE, "https://run.invalid/9"))

    def test_every_missing_piece_is_a_refusal(self) -> None:
        cases = {
            "the night failed": Fakes(self.module, conclusion="failure"),
            "the night did not set the alpha": Fakes(self.module, alpha_job="skipped"),
            "the night checked another commit": Fakes(self.module, checked="e" * 40),
            "the slice is not in main": Fakes(self.module, in_main=False),
            "the release names another slice": Fakes(self.module, body_slice="d" * 40),
        }
        for reason, fakes in cases.items():
            with self.subTest(reason=reason), self.assertRaises(self.module.RcError):
                self.prove(fakes)

    def test_a_tag_that_is_not_an_alpha_is_refused(self) -> None:
        with self.assertRaisesRegex(self.module.RcError, "не тег alpha"):
            self.module.prove("o/r", "v0.15.0-rc.1", Fakes(self.module).gh, lambda arguments: "")


class PublishTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def test_the_candidate_commit_is_cut_on_the_slice_and_the_tag_is_pushed_with_the_key(self) -> None:
        fakes = Fakes(self.module)
        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.dict(os.environ, {"RELEASE_TAG_KEY": "-----KEY-----", "GITHUB_OUTPUT": str(Path(directory) / "out")}):
            tag = self.module.publish(Path(directory), "o/r", ALPHA, dry_run=False, gh=fakes.gh, runner=fakes.runner)
            self.assertEqual((Path(directory) / "out").read_text(encoding="utf-8"), "tag=v0.15.0-rc.2\n")
        self.assertEqual(tag, "v0.15.0-rc.2")
        commands = [command for command, _ in fakes.commands]
        self.assertIn(["git", "checkout", "--detach", SLICE], commands)
        self.assertIn([sys.executable, "scripts/dev/bump-version.py", "0.15.0-rc.2"], commands)
        push = next((command, env) for command, env in fakes.commands if command[:2] == ["git", "push"])
        self.assertEqual(push[0], ["git", "push", "git@github.com:o/r.git", "refs/tags/v0.15.0-rc.2"])
        self.assertIn("StrictHostKeyChecking=yes", push[1]["GIT_SSH_COMMAND"])
        # The version commit goes on the slice before the tag.
        self.assertLess(commands.index(["git", "checkout", "--detach", SLICE]),
                        next(index for index, command in enumerate(commands) if command[:2] == ["git", "push"]))

    def test_a_dry_run_proves_and_names_the_number_without_touching_anything(self) -> None:
        fakes = Fakes(self.module)
        tag = self.module.publish(Path("."), "o/r", ALPHA, dry_run=True, gh=fakes.gh, runner=fakes.runner)
        self.assertEqual(tag, "v0.15.0-rc.2")
        self.assertFalse(any(command[:2] in (["git", "commit"], ["git", "push"], ["git", "checkout"])
                             for command, _ in fakes.commands))

    def test_without_the_key_nothing_is_pushed(self) -> None:
        fakes = Fakes(self.module)
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {"RELEASE_TAG_KEY": ""}), \
             self.assertRaisesRegex(self.module.RcError, "RELEASE_TAG_KEY"):
            self.module.publish(Path(directory), "o/r", ALPHA, dry_run=False, gh=fakes.gh, runner=fakes.runner)
        self.assertFalse(any(command[:2] == ["git", "push"] for command, _ in fakes.commands))


class WorkflowTests(unittest.TestCase):
    def test_a_button_on_main_in_the_release_tag_environment_with_the_tag_key(self) -> None:
        workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        self.assertEqual(set(workflow[True]), {"workflow_dispatch"})
        self.assertTrue(workflow[True]["workflow_dispatch"]["inputs"]["alpha"]["required"])
        self.assertEqual(workflow["permissions"], {"contents": "read", "actions": "read"})
        job = workflow["jobs"]["rc"]
        self.assertEqual(job["environment"], "release-tag")
        checkout = job["steps"][0]
        self.assertEqual((checkout["with"]["ref"], checkout["with"]["fetch-depth"]), ("main", 0))
        publish = next(step for step in job["steps"] if step.get("id") == "publish")
        self.assertEqual(publish["env"]["RELEASE_TAG_KEY"], "${{ secrets.RELEASE_TAG_KEY }}")
        self.assertIn('--alpha "$ALPHA_TAG"', publish["run"])


if __name__ == "__main__":
    unittest.main()
