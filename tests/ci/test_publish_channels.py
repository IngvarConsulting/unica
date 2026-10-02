"""Каналы публикации проверяются исполнением самих шагов workflow.

Шаги `stage` и `promote` берутся из publish-unica-marketplace.yml как есть и
выполняются над локальным репозиторием вместо маркетплейса: `gh repo clone`
клонирует его, а `git push` пишет в него. Так проверяется то, что увидят
потребители каналов, — ref каталогов на ветках `main` и `next`, — а не текст
шагов.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path

from tests.ci.test_release_channel import write_payload
from tests.ci.test_unica_workflow import PUBLISH_WORKFLOW, REPO_ROOT, job, load, step_named

CHANNEL_SCRIPT = REPO_ROOT / "scripts" / "ci" / "release-channel.py"
STAGE_STEP = ("stage", "Push the staged payload without changing any catalog")
PROMOTE_STEP = ("promote", "Move the channel catalogs to the published tag")
CODEX = ".agents/plugins/marketplace.json"
CLAUDE = ".claude-plugin/marketplace.json"
FAKE_GH = """#!/usr/bin/env bash
set -euo pipefail
if [ "$#" -eq 4 ] && [ "$1 $2 $3" = "repo clone IngvarConsulting/unica-marketplace" ]; then
  exec git clone -q "$FAKE_MARKETPLACE" "$4"
fi
echo "fake gh: unsupported: $*" >&2
exit 64
"""


def write_plugin(root: Path, version: str) -> None:
    for manifest in (".codex-plugin", ".claude-plugin", ".zcode-plugin"):
        path = root / "plugins" / "unica" / manifest / "plugin.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"name": "unica", "version": version}), encoding="utf-8")


class PublishChannelTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.publish = load(PUBLISH_WORKFLOW)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name, body in (("gh", FAKE_GH), ("python3", f'#!/bin/sh\nexec "{sys.executable}" "$@"\n')):
            path = self.bin / name
            path.write_text(body, encoding="utf-8")
            path.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "FAKE_MARKETPLACE": str(self.root / "marketplace.git"),
            # Подпись коммитов и прочие глобальные настройки пользователя не
            # должны попасть в опыт: он исполняет git так же, как раннер.
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": "test",
            "GIT_AUTHOR_EMAIL": "test@example.invalid",
            "GIT_COMMITTER_NAME": "test",
            "GIT_COMMITTER_EMAIL": "test@example.invalid",
        }
        seed = self.root / "seed"
        write_plugin(seed, "0.12.3")
        write_payload(seed, "v0.12.3")
        self.git("init", "-q", "-b", "main", cwd=seed)
        self.git("add", "-A", cwd=seed)
        self.git("commit", "-q", "-m", "promote: Unica v0.12.3", cwd=seed)
        self.git("clone", "-q", "--bare", str(seed), str(self.root / "marketplace.git"))
        self.runs = 0

    def tearDown(self) -> None:
        self.temp.cleanup()

    def git(self, *args: str, cwd: Path | None = None) -> str:
        return subprocess.run(
            ["git", *args], cwd=cwd, env=self.env, check=True, capture_output=True, text=True
        ).stdout

    def marketplace(self, *args: str) -> str:
        return self.git("--git-dir", str(self.root / "marketplace.git"), *args)

    def catalog(self, branch: str, path: str) -> dict:
        return json.loads(self.marketplace("show", f"{branch}:{path}"))

    def served(self, branch: str) -> tuple[str, str, str]:
        """(marketplace name, codex ref, claude ref) the branch serves."""
        codex, claude = self.catalog(branch, CODEX), self.catalog(branch, CLAUDE)
        self.assertEqual(codex["name"], claude["name"])
        return codex["name"], codex["plugins"][0]["source"]["ref"], claude["plugins"][0]["source"]["ref"]

    def branches(self) -> set[str]:
        return {line.strip().removeprefix("* ") for line in self.marketplace("branch").splitlines()}

    def run_step(self, step: tuple[str, str], tag: str, channel: str) -> subprocess.CompletedProcess:
        self.runs += 1
        workspace = self.root / f"run-{self.runs}"
        rules = workspace / "rules" / "scripts" / "ci"
        rules.mkdir(parents=True)
        shutil.copy(CHANNEL_SCRIPT, rules / CHANNEL_SCRIPT.name)
        write_plugin(workspace / "payload", tag.removeprefix("v"))
        write_payload(workspace / "payload", tag)
        output = workspace / "github-output"
        output.touch()
        env = {**self.env, "RELEASE_TAG": tag, "CHANNEL": channel, "GITHUB_OUTPUT": str(output)}
        text = step_named(job(self.publish, step[0]), step[1])["run"]
        return subprocess.run(["bash", "-c", text], cwd=workspace, env=env, capture_output=True, text=True)

    def release(self, tag: str, channel: str) -> None:
        for step in (STAGE_STEP, PROMOTE_STEP):
            result = self.run_step(step, tag, channel)
            self.assertEqual(result.returncode, 0, f"{step[0]} {tag}: {result.stderr}")

    def heads(self) -> dict[str, str]:
        return {branch: self.marketplace("rev-parse", branch) for branch in sorted(self.branches())}

    def plugin_version(self, branch: str) -> str:
        versions = [
            json.loads(self.marketplace("show", f"{branch}:plugins/unica/{manifest}/plugin.json"))["version"]
            for manifest in (".codex-plugin", ".claude-plugin", ".zcode-plugin")
        ]
        self.assertEqual(len(set(versions)), 1, versions)
        return versions[0]

    def change_main(self, message: str, edit) -> None:
        """Commit an unrelated maintenance change to the marketplace main branch."""
        work = self.root / f"main-{self.runs}-{abs(hash(message))}"
        self.git("clone", "-q", "-b", "main", str(self.root / "marketplace.git"), str(work))
        edit(work)
        self.git("add", "-A", cwd=work)
        self.git("commit", "-q", "-m", message, cwd=work)
        self.git("push", "-q", "origin", "main", cwd=work)

    def lock_main(self) -> Path:
        """A pre-receive hook refuses pushes to main while the returned flag exists."""
        flag = self.root / "main-locked"
        hook = self.root / "marketplace.git" / "hooks" / "pre-receive"
        hook.write_text(
            "#!/bin/sh\n"
            "while read old new ref; do\n"
            f'  if [ "$ref" = refs/heads/main ] && [ -f "{flag}" ]; then echo "main is locked" >&2; exit 1; fi\n'
            "done\n",
            encoding="utf-8",
        )
        hook.chmod(0o755)
        flag.touch()
        return flag

    def test_a_candidate_reaches_only_the_next_channel(self) -> None:
        main_before = self.marketplace("rev-parse", "main")

        self.release("v0.13.0-rc.3", "next")

        self.assertEqual(self.marketplace("rev-parse", "main"), main_before)
        self.assertEqual(self.served("main"), ("unica", "v0.12.3", "v0.12.3"))
        self.assertEqual(self.served("next"), ("unica-next", "v0.13.0-rc.3", "v0.13.0-rc.3"))
        self.assertEqual(self.plugin_version("next"), "0.13.0-rc.3")
        # Канал рождается копией основного, а не пустым: до первого кандидата
        # его подписчики получают действующий стабильный выпуск.
        history = self.marketplace("log", "--format=%s", "next").splitlines()
        self.assertIn("next: start from main at v0.12.3", history)

    def test_a_release_reaches_both_channels_after_its_candidate(self) -> None:
        self.release("v0.13.0-rc.3", "next")
        self.release("v0.13.0", "stable")

        self.assertEqual(self.served("main"), ("unica", "v0.13.0", "v0.13.0"))
        self.assertEqual(self.served("next"), ("unica-next", "v0.13.0", "v0.13.0"))
        # next carries the bytes it serves, so its tree passes the same checks as main.
        self.assertEqual(self.plugin_version("next"), "0.13.0")

    def test_a_stable_release_opens_the_next_channel_when_none_exists(self) -> None:
        self.release("v0.12.4", "stable")

        self.assertEqual(self.served("main"), ("unica", "v0.12.4", "v0.12.4"))
        self.assertEqual(self.served("next"), ("unica-next", "v0.12.4", "v0.12.4"))

    def test_an_older_stable_release_leaves_the_newer_candidate_in_next(self) -> None:
        self.release("v0.13.0", "stable")
        self.release("v0.14.0-rc.1", "next")
        self.release("v0.13.1", "stable")

        self.assertEqual(self.served("main"), ("unica", "v0.13.1", "v0.13.1"))
        self.assertEqual(self.served("next"), ("unica-next", "v0.14.0-rc.1", "v0.14.0-rc.1"))

    def test_a_candidate_older_than_a_served_release_is_refused(self) -> None:
        self.release("v0.13.0", "stable")
        before = {branch: self.marketplace("rev-parse", branch) for branch in ("main", "next")}

        result = self.run_step(STAGE_STEP, "v0.13.0-rc.4", "next")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("older than v0.13.0", result.stderr)
        self.assertEqual({branch: self.marketplace("rev-parse", branch) for branch in before}, before)

    def test_a_rerun_of_a_completed_publication_changes_nothing(self) -> None:
        self.release("v0.13.0-rc.3", "next")
        before = {branch: self.marketplace("rev-parse", branch) for branch in ("main", "next")}

        self.release("v0.13.0-rc.3", "next")

        self.assertEqual({branch: self.marketplace("rev-parse", branch) for branch in before}, before)

    def test_a_stale_stable_release_is_refused_at_stage(self) -> None:
        self.release("v0.13.0", "stable")
        before = self.heads()

        result = self.run_step(STAGE_STEP, "v0.12.4", "stable")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("older than v0.13.0", result.stderr)
        self.assertEqual(self.heads(), before)

    def test_a_candidate_older_than_next_is_refused_at_stage(self) -> None:
        # main still serves v0.12.3: only the check against next stops rc.2.
        self.release("v0.13.0-rc.3", "next")
        before = self.heads()

        result = self.run_step(STAGE_STEP, "v0.13.0-rc.2", "next")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("older than v0.13.0-rc.3", result.stderr)
        self.assertEqual(self.heads(), before)

    def test_next_moves_before_main_and_a_rerun_completes_the_release(self) -> None:
        self.release("v0.13.0-rc.3", "next")
        self.assertEqual(self.run_step(STAGE_STEP, "v0.13.0", "stable").returncode, 0)
        flag = self.lock_main()

        failed = self.run_step(PROMOTE_STEP, "v0.13.0", "stable")

        self.assertNotEqual(failed.returncode, 0)
        # The failure between the two pushes leaves next ahead of main, never behind.
        self.assertEqual(self.served("next"), ("unica-next", "v0.13.0", "v0.13.0"))
        self.assertEqual(self.served("main"), ("unica", "v0.12.3", "v0.12.3"))

        flag.unlink()
        rerun = self.run_step(PROMOTE_STEP, "v0.13.0", "stable")

        self.assertEqual(rerun.returncode, 0, rerun.stderr)
        self.assertEqual(self.served("main"), ("unica", "v0.13.0", "v0.13.0"))
        self.assertEqual(self.served("next"), ("unica-next", "v0.13.0", "v0.13.0"))

    def test_next_follows_main_outside_the_served_plugin(self) -> None:
        # The marketplace checks its branches with the scripts and workflows
        # the branch carries; next must not keep the ones it was born with.
        def checks(version: str, *, retired: bool) -> Callable[[Path], None]:
            def edit(work: Path) -> None:
                (work / "checks").mkdir(exist_ok=True)
                (work / "checks" / "verify.txt").write_text(version, encoding="utf-8")
                retired_file = work / "checks" / "retired.txt"
                if retired:
                    retired_file.unlink(missing_ok=True)
                else:
                    retired_file.write_text("old", encoding="utf-8")
            return edit

        self.change_main("add checks", checks("v1", retired=False))
        self.release("v0.13.0-rc.3", "next")
        self.change_main("update checks", checks("v2", retired=True))

        # A candidate is staged, tagged and checked on a tree that already follows main.
        staged = self.run_step(STAGE_STEP, "v0.13.0-rc.4", "next")
        self.assertEqual(staged.returncode, 0, staged.stderr)
        output = (self.root / f"run-{self.runs}" / "github-output").read_text(encoding="utf-8")
        staging_sha = output.split("staging_sha=", 1)[1].split()[0]
        self.assertEqual(self.marketplace("show", f"{staging_sha}:checks/verify.txt"), "v2")
        self.assertNotIn("checks/retired.txt", self.marketplace("ls-tree", "-r", "--name-only", staging_sha))
        promoted = self.run_step(PROMOTE_STEP, "v0.13.0-rc.4", "next")
        self.assertEqual(promoted.returncode, 0, promoted.stderr)

        # A stable release stages on main only, so promote alone brings next along.
        self.change_main("update checks again", checks("v3", retired=True))
        self.release("v0.13.0", "stable")

        self.assertEqual(self.marketplace("show", "next:checks/verify.txt"), "v3")
        self.assertEqual(self.served("next"), ("unica-next", "v0.13.0", "v0.13.0"))
        self.assertEqual(self.plugin_version("next"), "0.13.0")
        self.assertEqual(self.served("main"), ("unica", "v0.13.0", "v0.13.0"))

    def test_host_catalogs_that_disagree_stop_the_publication(self) -> None:
        self.release("v0.13.0-rc.3", "next")
        work = self.root / "tamper"
        self.git("clone", "-q", "-b", "next", str(self.root / "marketplace.git"), str(work))
        claude = work / CLAUDE
        catalog = json.loads(claude.read_text(encoding="utf-8"))
        catalog["plugins"][0]["source"]["ref"] = "v0.12.3"
        claude.write_text(json.dumps(catalog), encoding="utf-8")
        self.git("commit", "-q", "-am", "diverge", cwd=work)
        self.git("push", "-q", "origin", "next", cwd=work)
        before = self.marketplace("rev-parse", "next")

        result = self.run_step(PROMOTE_STEP, "v0.13.0-rc.4", "next")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("host catalogs disagree", result.stderr)
        self.assertEqual(self.marketplace("rev-parse", "next"), before)
        self.assertEqual(self.branches(), {"main", "next"})


if __name__ == "__main__":
    unittest.main()
