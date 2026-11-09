"""Исправительная версия из линии: доказательство, номер, служебный коммит, тег ключом и PR обратно в main."""

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
WORKFLOW = ROOT / ".github/workflows/release-from-line.yml"
LINE = "release-v0.15"
HEAD = "h" * 40
FINAL, FINAL_PARENT = "f" * 40, "e" * 40
PATCH1, PATCH1_PARENT = "p" * 40, "q" * 40


def load():
    spec = importlib.util.spec_from_file_location("release_patch", ROOT / "scripts/ci/release-patch.py")
    module = importlib.util.module_from_spec(spec)
    # `dataclass` ищет модуль класса в `sys.modules`.
    sys.modules["release_patch"] = module
    spec.loader.exec_module(module)
    return module


class Fakes:
    def __init__(self, module, *, tags=("v0.15.0", "v0.15.1", "v0.16.0-rc.1", "v0.14.3"), head=HEAD,
                 runs=None, line_exists=True):
        self.module = module
        self.tags, self.head, self.line_exists = tags, head, line_exists
        self.runs = runs if runs is not None else [
            {"head_sha": head, "conclusion": "success", "path": ".github/workflows/ci.yml",
             "created_at": "2026-11-09", "html_url": "https://run.invalid/5"}]
        self.commands = []
        self.commits = {"v0.15.0": FINAL, "v0.15.1": PATCH1}
        self.parents = {FINAL: FINAL_PARENT, PATCH1: PATCH1_PARENT}

    def gh(self, path, **_):
        if "matching-refs/tags/v" in path:
            return [{"ref": f"refs/tags/{tag}"} for tag in self.tags]
        if path.startswith("repos/o/r/actions/runs?"):
            self.runs_path = path
            return {"workflow_runs": self.runs}
        raise AssertionError(path)

    def runner(self, command, cwd=None, env=None):
        self.commands.append((command, env))
        if command[:2] == ["git", "rev-parse"]:
            target = command[-1]
            if target == f"refs/remotes/origin/{LINE}^{{commit}}":
                if not self.line_exists:
                    raise self.module.alpha.AlphaError("unknown revision")
                return self.head + "\n"
            if target.endswith("^{commit}"):
                return self.commits[target.removesuffix("^{commit}")] + "\n"
            if target.endswith("^"):
                return self.parents[target.removesuffix("^")] + "\n"
        return ""


class ProofTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def prove(self, fakes, line=LINE):
        return self.module.prove("o/r", line, fakes.gh, lambda arguments: fakes.runner(["git", *arguments]))

    def test_a_green_unreleased_head_of_the_line_gets_the_next_patch(self) -> None:
        fakes = Fakes(self.module)
        proof = self.prove(fakes)
        self.assertEqual((proof.line, proof.head, proof.tag, proof.run_url),
                         (LINE, HEAD, "v0.15.2", "https://run.invalid/5"))
        self.assertIn(f"head_sha={HEAD}", fakes.runs_path)

    def test_a_check_of_the_line_by_the_release_button_counts_as_its_ci(self) -> None:
        runs = [{"head_sha": HEAD, "conclusion": "success",
                 "path": ".github/workflows/release.yml@refs/heads/release-v0.15", "html_url": "u"}]
        self.assertEqual(self.prove(Fakes(self.module, runs=runs)).tag, "v0.15.2")

    def test_every_missing_piece_is_a_refusal(self) -> None:
        other = {"head_sha": HEAD, "conclusion": "success", "path": ".github/workflows/pr-metadata.yml"}
        cases = {
            "только из ветки release-vX.Y": (Fakes(self.module), "main"),
            "нет финальной версии": (Fakes(self.module, tags=("v0.15.0-rc.2", "v0.14.3")), LINE),
            "ветки release-v0.15 нет": (Fakes(self.module, line_exists=False), LINE),
            "уже выпущена как v0.15.0": (Fakes(self.module, head=FINAL), LINE),
            "уже выпущена как v0.15.1": (Fakes(self.module, head=PATCH1_PARENT), LINE),
            "нет зелёного CI": (Fakes(self.module, runs=[]), LINE),
            "нет зелёного CI ": (Fakes(self.module, runs=[{**other, "path": ".github/workflows/ci.yml",
                                                           "conclusion": "failure"}]), LINE),
            "нет зелёного CI  ": (Fakes(self.module, runs=[other]), LINE),
        }
        for reason, (fakes, line) in cases.items():
            with self.subTest(reason=reason), self.assertRaisesRegex(self.module.PatchError, reason.strip()):
                self.prove(fakes, line)


class PublishTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def test_the_version_commit_is_cut_on_the_head_and_only_the_tag_is_pushed(self) -> None:
        fakes = Fakes(self.module)
        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.dict(os.environ, {"RELEASE_TAG_KEY": "-----KEY-----", "GITHUB_OUTPUT": str(Path(directory) / "out")}):
            tag = self.module.publish(Path(directory), "o/r", LINE, dry_run=False, gh=fakes.gh, runner=fakes.runner)
            self.assertEqual((Path(directory) / "out").read_text(encoding="utf-8"), "tag=v0.15.2\n")
        self.assertEqual(tag, "v0.15.2")
        commands = [command for command, _ in fakes.commands]
        checkout = commands.index(["git", "checkout", "--detach", HEAD])
        bump = commands.index([sys.executable, "scripts/dev/bump-version.py", "0.15.2"])
        push = next(index for index, command in enumerate(commands) if command[:2] == ["git", "push"])
        self.assertLess(checkout, bump)
        self.assertLess(bump, push)
        # Служебный коммит живёт только под тегом: голова линии не двигается.
        self.assertEqual(commands[push], ["git", "push", "git@github.com:o/r.git", "refs/tags/v0.15.2"])
        self.assertIn("StrictHostKeyChecking=yes", fakes.commands[push][1]["GIT_SSH_COMMAND"])

    def test_a_dry_run_names_the_version_without_touching_anything(self) -> None:
        fakes = Fakes(self.module)
        self.assertEqual(self.module.publish(Path("."), "o/r", LINE, dry_run=True, gh=fakes.gh, runner=fakes.runner),
                         "v0.15.2")
        self.assertFalse(any(command[0] == "git" and {"commit", "tag", "push", "checkout"} & set(command)
                             for command, _ in fakes.commands))

    def test_without_the_key_nothing_is_pushed(self) -> None:
        fakes = Fakes(self.module)
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {"RELEASE_TAG_KEY": ""}), \
             self.assertRaisesRegex(self.module.PatchError, "RELEASE_TAG_KEY"):
            self.module.publish(Path(directory), "o/r", LINE, dry_run=False, gh=fakes.gh, runner=fakes.runner)
        self.assertFalse(any(command[:2] == ["git", "push"] for command, _ in fakes.commands))


class BackMergeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def merge(self, *, conflicted="", staged="crates/x/src/lib.rs\n", open_branch="", merged_in=False,
              started=True, push_refused=False):
        commands, probes = [], []
        module = self.module

        def runner(command, cwd=None, env=None):
            commands.append(command)
            if command[:3] == ["gh", "pr", "create"]:
                return "https://github.com/o/r/pull/9\n"
            if command[:2] == ["git", "rev-parse"]:
                return "linehead\n"
            if command[:2] == ["git", "push"] and push_refused:
                raise module.alpha.AlphaError("refusing to allow a GitHub App to create or update workflow")
            return ""

        def probe(command, cwd=None):
            probes.append(command)
            if command[:2] == ["git", "ls-remote"]:
                return open_branch
            if command[:2] == ["git", "merge-base"]:
                return "linehead\n" if merged_in else "older\n"
            if command[-1] == "MERGE_HEAD":
                return "mergehead\n" if started else ""
            if "--diff-filter=U" in command:
                return conflicted
            if command[-2:] == ["--name-only", "origin/main"]:
                return staged
            return ""

        return commands, probes, runner, probe

    def back_merge(self, runner, probe, tag="v0.15.2"):
        return self.module.back_merge(Path("."), "o/r", tag, runner=runner, probe=probe)

    @staticmethod
    def pushed(commands):
        return any(command[:2] == ["git", "push"] or command[:2] == ["gh", "pr"] for command in commands)

    def test_the_line_returns_to_main_with_mains_side_of_the_version(self) -> None:
        commands, probes, runner, probe = self.merge(conflicted="Cargo.toml\nCargo.lock\n")
        branch = self.back_merge(runner, probe)
        self.assertEqual(branch, "merge/release-v0.15.2-into-main")
        self.assertIn(["git", "checkout", "-B", branch, "origin/main"], commands)
        self.assertIn(["git", "checkout", "origin/main", "--", *self.module.VERSION_FILES], commands)
        self.assertIn("origin/release-v0.15", next(command for command in probes if "--no-commit" in command))
        push = next(command for command in commands if command[:2] == ["git", "push"])
        self.assertLess(commands.index(["gh", "auth", "setup-git"]), commands.index(push))
        self.assertEqual(push, ["git", "push", "origin", f"HEAD:refs/heads/{branch}"])
        create = next(command for command in commands if command[:3] == ["gh", "pr", "create"])
        self.assertEqual(create[create.index("--base") + 1], "main")
        self.assertIn("координатор", create[create.index("--body") + 1])
        self.assertEqual(len(self.module.VERSION_FILES), 6)

    def test_a_conflict_outside_the_version_is_left_to_a_person(self) -> None:
        commands, probes, runner, probe = self.merge(conflicted="Cargo.toml\ncrates/x/src/lib.rs\n")
        with self.assertRaisesRegex(self.module.PatchError, "crates/x/src/lib.rs"):
            self.back_merge(runner, probe)
        self.assertIn(["git", "merge", "--abort"], probes)
        self.assertFalse(self.pushed(commands))

    def test_a_line_already_in_main_opens_nothing(self) -> None:
        commands, probes, runner, probe = self.merge(merged_in=True)
        self.assertIsNone(self.back_merge(runner, probe))
        self.assertFalse(any("--no-commit" in command for command in probes))
        self.assertFalse(self.pushed(commands))

    def test_a_line_that_differs_only_in_the_version_opens_nothing(self) -> None:
        commands, _, runner, probe = self.merge(conflicted="Cargo.toml\n", staged="")
        self.assertIsNone(self.back_merge(runner, probe))
        self.assertFalse(self.pushed(commands))

    def test_a_merge_that_did_not_start_is_a_failure_not_an_empty_line(self) -> None:
        commands, _, runner, probe = self.merge(started=False)
        with self.assertRaisesRegex(self.module.PatchError, "не началось"):
            self.back_merge(runner, probe)
        self.assertFalse(self.pushed(commands))

    def test_a_rerun_leaves_the_branch_of_an_earlier_run_alone(self) -> None:
        commands, _, runner, probe = self.merge(open_branch="abc\trefs/heads/merge/release-v0.15.2-into-main\n")
        self.assertEqual(self.back_merge(runner, probe), "merge/release-v0.15.2-into-main")
        self.assertEqual(commands, [])

    def test_a_refused_push_names_the_workflow_files_and_the_way_by_hand(self) -> None:
        commands, _, runner, probe = self.merge(push_refused=True)
        with self.assertRaisesRegex(self.module.PatchError, r"\.github/workflows.*вручную"):
            self.back_merge(runner, probe)
        self.assertFalse(any(command[:2] == ["gh", "pr"] for command in commands))

    def test_only_a_stable_tag_is_merged_back(self) -> None:
        _, _, runner, probe = self.merge()
        with self.assertRaisesRegex(self.module.PatchError, "не тег исправительной"):
            self.back_merge(runner, probe, "v0.15.2-rc.1")

    def test_the_version_files_are_the_version_contract_and_the_lock(self) -> None:
        source = (ROOT / "scripts/dev/bump-version.py").read_text(encoding="utf-8")
        for path in self.module.VERSION_FILES[:-1]:
            *_, name = path.split("/")
            with self.subTest(path=path):
                self.assertIn(f'"{name}"', source)


class WorkflowTests(unittest.TestCase):
    def test_only_an_administrator_on_main_reaches_the_key_and_the_line_returns_to_main_after(self) -> None:
        workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        self.assertEqual(set(workflow[True]), {"workflow_dispatch"})
        self.assertTrue(workflow[True]["workflow_dispatch"]["inputs"]["line"]["required"])
        self.assertEqual(workflow["permissions"], {"contents": "read", "actions": "read"})
        command = 'release-rc.py admit --actor "$ACTOR" --ref "$GITHUB_REF"'
        jobs = workflow["jobs"]
        self.assertNotIn("environment", jobs["admit"])
        self.assertNotIn("secrets.", json.dumps(jobs["admit"]))
        patch = jobs["patch"]
        self.assertEqual((patch["needs"], patch["environment"]), ("admit", "alpha"))
        steps = patch["steps"]
        recheck = next(index for index, step in enumerate(steps) if step.get("id") == "admit")
        publish = next(index for index, step in enumerate(steps) if step.get("id") == "publish")
        self.assertLess(recheck, publish)
        self.assertIn(command, steps[recheck]["run"])
        self.assertNotIn("RELEASE_TAG_KEY", json.dumps(steps[:publish]))
        self.assertIn('release-patch.py publish --line "$LINE"', steps[publish]["run"])
        back = jobs["back-merge"]
        self.assertEqual(back["needs"], "patch")
        self.assertNotIn("environment", back)
        self.assertNotIn("secrets.", json.dumps(back))
        self.assertEqual(back["permissions"], {"contents": "write", "pull-requests": "write"})
        self.assertIs(back["steps"][0]["with"]["persist-credentials"], False)
        self.assertIn('release-patch.py back-merge --tag "$TAG"', back["steps"][-1]["run"])
        self.assertEqual(back["steps"][-1]["env"]["TAG"], "${{ needs.patch.outputs.tag }}")


if __name__ == "__main__":
    unittest.main()
