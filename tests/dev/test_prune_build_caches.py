"""Tests for the cleanup of Cargo build directories and working copies.

Directories are real temporary ones; `gh`, `git` and the process list come
from a fake ``System`` so the tests need no network, repository or builds.
"""

from __future__ import annotations

import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "prune-build-caches.py"
SPEC = importlib.util.spec_from_file_location("prune_build_caches", SCRIPT)
PRUNE = importlib.util.module_from_spec(SPEC)
sys.modules["prune_build_caches"] = PRUNE
SPEC.loader.exec_module(PRUNE)

NOW = 2_000_000_000.0
DAY = 24 * 60 * 60


class FakeSystem(PRUNE.System):
    def __init__(self, repo: Path) -> None:
        self.system = "Linux"
        self.repo = repo
        self.worktrees: list = []  # (path, branch or None, locked)
        self.prs: dict = {}  # branch -> list of {number, state, headRefOid}
        self.pr_states: dict = {}  # number -> state
        self.dirty: set = set()
        self.unpushed: set = set()
        self.not_ancestor: set = set()
        self.commands: list = []
        self.calls: list = []
        self.removed: list = []

    def now(self):
        return NOW

    def process_command_lines(self):
        return self.commands

    def run(self, argv, cwd=None):
        argv = list(argv)
        self.calls.append(argv)
        if argv[:2] == ["gh", "pr"] and argv[2] == "list":
            selector = "--head" if "--head" in argv else "--search"
            return 0, json.dumps(self.prs.get(argv[argv.index(selector) + 1], []))
        if argv[:3] == ["gh", "pr", "view"]:
            state = self.pr_states.get(int(argv[3]))
            return (0, json.dumps({"state": state})) if state else (1, "no pull requests found")
        if argv[0] == "git":
            where, rest = Path(argv[2]), argv[3:]
            if rest[:2] == ["worktree", "list"]:
                blocks = [f"worktree {self.repo}\nHEAD 0\nbranch refs/heads/main\n"]
                for path, branch, locked in self.worktrees:
                    block = f"worktree {path}\nHEAD 1\n"
                    block += f"branch refs/heads/{branch}\n" if branch else "detached\n"
                    block += "locked\n" if locked else ""
                    blocks.append(block)
                return 0, "\n".join(blocks)
            if rest[:2] == ["worktree", "remove"]:
                self.removed.append(Path(rest[2]))
                return 0, ""
            if rest[:2] == ["rev-parse", "HEAD"]:
                return 0, f"sha-{where.name}\n"
            if rest[0] == "status":
                return 0, " M file.rs\n" if where in self.dirty else ""
            if rest[0] == "merge-base":
                return (1, "") if where in self.not_ancestor else (0, "")
            if rest[0] == "rev-list":
                return 0, "2\n" if where in self.unpushed else "0\n"
        return 127, "unexpected " + " ".join(argv)

    def remove_tree(self, path):
        self.removed.append(Path(path))
        super().remove_tree(path)


def make_dir(path: Path, age_days: float, size: int = 1024) -> Path:
    (path / "debug" / "deps").mkdir(parents=True, exist_ok=True)
    (path / "debug" / "deps" / "lib.rlib").write_bytes(b"x" * size)
    moment = NOW - age_days * DAY
    for directory, _, files in os.walk(path):
        for name in files:
            os.utime(Path(directory) / name, (moment, moment))
        os.utime(directory, (moment, moment))
    return path


class PruneBuildCachesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.builds = self.root / "cargo-target"
        self.builds.mkdir()
        self.system = FakeSystem(self.repo)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def prune(self, *args: str):
        out = io.StringIO()
        with redirect_stdout(out):
            code = PRUNE.main(["--build-root", str(self.builds), "--json", *args], self.system, self.repo)
        return code, json.loads(out.getvalue())

    def reasons(self, report) -> dict:
        return {Path(c["path"]).name: c["reason"] for c in report["candidates"]}

    def add_worktree(self, name: str, branch, age_days: float = 0, locked: bool = False) -> Path:
        path = make_dir(self.root / name, age_days)
        self.system.worktrees.append((path, branch, locked))
        return path

    def test_build_dir_of_a_merged_or_closed_pr_is_a_candidate(self) -> None:
        make_dir(self.builds / "worker-1-2399", 0)
        make_dir(self.builds / "worker-3-2310", 0)
        make_dir(self.builds / "worker-3-2277", 0)
        self.system.pr_states = {2399: "MERGED", 2310: "CLOSED", 2277: "OPEN"}
        _, report = self.prune()
        self.assertEqual(self.reasons(report), {"worker-1-2399": "PR #2399 merged", "worker-3-2310": "PR #2310 closed"})

    def test_build_dir_unused_for_longer_than_the_limit_is_abandoned(self) -> None:
        make_dir(self.builds / "worker-2", 5)
        make_dir(self.builds / "worker-2b", 1)
        _, report = self.prune()
        self.assertEqual(list(self.reasons(report)), ["worker-2"])
        _, report = self.prune("--stale-days", "0.5")
        self.assertEqual(sorted(self.reasons(report)), ["worker-2", "worker-2b"])

    def test_directory_named_by_a_running_process_is_kept(self) -> None:
        busy = make_dir(self.builds / "worker-1-2399", 9)
        self.system.pr_states = {2399: "MERGED"}
        self.system.commands = [f"{busy}/debug/deps/unica_daemon-0a1b --exact medium_tests::x"]
        _, report = self.prune()
        self.assertEqual(report["candidates"], [])

    def test_windows_process_match_ignores_case_and_separators(self) -> None:
        path = Path(r"C:\cargo-target\worker-2")
        lines = [r"c:\CARGO-TARGET\worker-2\debug\deps\unica.exe --exact t"]
        self.assertTrue(PRUNE.in_use(path, lines, "Windows"))
        self.assertFalse(PRUNE.in_use(Path(r"C:\cargo-target\worker"), lines, "Windows"))
        shell = ["bash -c 'export CARGO_TARGET_DIR=C:/cargo-target/worker-1; cargo test'"]
        self.assertTrue(PRUNE.in_use(Path(r"C:\cargo-target\worker-1"), shell, "Windows"))
        self.assertFalse(PRUNE.in_use(Path(r"C:\cargo-target\worker-1-pair"), shell, "Windows"))

    def test_working_copy_of_a_merged_pr_is_a_candidate(self) -> None:
        self.add_worktree("w-merged", "claude/merged")
        self.system.prs = {"claude/merged": [{"number": 2477, "state": "MERGED", "headRefOid": "abc"}]}
        _, report = self.prune()
        self.assertEqual(self.reasons(report), {"w-merged": "PR #2477 merged"})

    def test_copy_of_someone_elses_pr_is_found_by_its_commit(self) -> None:
        self.add_worktree("w-fork", "pr-2265")
        self.add_worktree("w-detached", None)
        self.system.prs = {
            "sha-w-fork": [{"number": 2265, "state": "MERGED", "headRefOid": "f"}],
            "sha-w-detached": [{"number": 2266, "state": "CLOSED", "headRefOid": "g"}],
        }
        _, report = self.prune()
        self.assertEqual(self.reasons(report), {"w-fork": "PR #2265 merged", "w-detached": "PR #2266 closed"})

    def test_working_copy_with_open_pr_or_local_work_is_kept(self) -> None:
        self.add_worktree("w-open", "claude/open", age_days=30)
        self.system.prs["claude/open"] = [
            {"number": 1, "state": "CLOSED", "headRefOid": "a"},
            {"number": 2, "state": "OPEN", "headRefOid": "b"},
        ]
        dirty = self.add_worktree("w-dirty", "claude/dirty")
        after = self.add_worktree("w-after", "claude/after")
        self.add_worktree("w-locked", "claude/locked", locked=True)
        for branch in ("claude/dirty", "claude/after", "claude/locked"):
            self.system.prs[branch] = [{"number": 3, "state": "MERGED", "headRefOid": "c"}]
        self.system.dirty.add(dirty)
        self.system.not_ancestor.add(after)
        _, report = self.prune()
        self.assertEqual(report["candidates"], [])

    def test_abandoned_working_copy_needs_no_unpushed_commits(self) -> None:
        self.add_worktree("w-old", None, age_days=10)
        kept = self.add_worktree("w-unpushed", "claude/local", age_days=10)
        self.system.unpushed.add(kept)
        _, report = self.prune()
        self.assertEqual(list(self.reasons(report)), ["w-old"])
        self.assertIn("unchanged for more than 3 days", report["candidates"][0]["reason"])

    def test_target_inside_an_active_working_copy_is_a_candidate(self) -> None:
        copy = self.add_worktree("w-active", "claude/active")
        make_dir(copy / "target", 0)
        self.system.prs["claude/active"] = [{"number": 5, "state": "OPEN", "headRefOid": "d"}]
        _, report = self.prune()
        self.assertEqual(report["candidates"][0]["kind"], "in-copy target")
        self.assertEqual(Path(report["candidates"][0]["path"]), copy / "target")

    def test_main_checkout_is_never_a_candidate(self) -> None:
        make_dir(self.repo, 30)
        _, report = self.prune()
        self.assertEqual(report["candidates"], [])

    def test_default_run_only_lists(self) -> None:
        stale = make_dir(self.builds / "worker-2", 5, size=4096)
        copy = self.add_worktree("w-merged", "claude/merged")
        self.system.prs = {"claude/merged": [{"number": 7, "state": "MERGED", "headRefOid": "e"}]}
        out = io.StringIO()
        with redirect_stdout(out):
            code = PRUNE.main(["--build-root", str(self.builds)], self.system, self.repo)
        self.assertEqual(code, 0)
        self.assertIn("run again with --apply to delete", out.getvalue())
        self.assertEqual(self.system.removed, [])
        self.assertTrue(stale.is_dir() and copy.is_dir())

    def test_apply_removes_build_dirs_and_working_copies_through_git(self) -> None:
        stale = make_dir(self.builds / "worker-2", 5)
        os.chmod(stale / "debug" / "deps" / "lib.rlib", 0o444)
        copy = self.add_worktree("w-merged", "claude/merged")
        self.system.prs = {"claude/merged": [{"number": 7, "state": "MERGED", "headRefOid": "e"}]}
        code, report = self.prune("--apply")
        self.assertEqual(code, 0)
        self.assertFalse(stale.exists())
        self.assertIn(copy, self.system.removed)
        self.assertIn(["git", "-C", str(self.repo), "worktree", "remove", str(copy)], self.system.calls)
        self.assertTrue(all(c["removed"] for c in report["candidates"]))


if __name__ == "__main__":
    unittest.main()
