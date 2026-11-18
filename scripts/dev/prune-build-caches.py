#!/usr/bin/env python3
"""Find Cargo build directories and working copies that can be deleted.

A candidate is one of:

- a working copy whose branch has a merged or closed PR, with no local
  changes and no commits beyond that PR's head;
- a build directory named `<name>-<PR number>` whose PR is merged or closed;
- an abandoned build directory or working copy: no running process mentions
  it and nothing in it changed for `--stale-days` days;
- a `target/` directory inside a working copy, which conveyor copies must not
  keep, when no running process mentions it.

A working copy with uncommitted changes, unpushed commits or a lock is never
a candidate, and neither is a symlink or junction. A merged PR's build
directory and an in-copy `target/` must also be unchanged for an hour: cargo
reads its build directory from the environment, so a running cargo is seen
only through the files it writes. Nothing is deleted without `--apply`; the
default run only prints. PR states come from `gh`; if `gh` or the process
listing fails, the script stops without deleting anything.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shutil
import stat
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple


REPO_ROOT = Path(__file__).resolve().parents[2]
GIB = 1024 ** 3
DAY = 24 * 60 * 60
DEFAULT_STALE_DAYS = 3
# `worker-1-2399`: the trailing number is the PR the directory was built for.
PR_SUFFIX = re.compile(r"-(\d{3,})$")
FINISHED = ("MERGED", "CLOSED")
GRACE_SECONDS = 60 * 60
# FILE_ATTRIBUTE_REPARSE_POINT: junctions and symlinks on Windows.
REPARSE_POINT = 0x400


class PruneError(Exception):
    """A lookup failed; deleting on a partial picture is not safe."""


@dataclass
class Candidate:
    kind: str  # "build dir", "working copy" or "in-copy target"
    path: str
    reason: str
    size: int


@dataclass
class Worktree:
    path: Path
    branch: Optional[str]
    locked: bool


class System:
    """Everything the script reads or changes outside its own logic; tests replace it."""

    def __init__(self) -> None:
        self.system = platform.system()

    def run(self, argv: Sequence[str], cwd: Optional[Path] = None) -> Tuple[int, str]:
        try:
            completed = subprocess.run(
                list(argv), cwd=str(cwd) if cwd else None,
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120,
            )
        except (OSError, subprocess.SubprocessError) as error:
            return 127, str(error)
        out = completed.stdout.decode("utf-8", "replace")
        if completed.returncode != 0:
            out += completed.stderr.decode("utf-8", "replace")
        return completed.returncode, out

    def now(self) -> float:
        return time.time()

    def process_command_lines(self) -> List[str]:
        """Executable paths and command lines of running processes.

        Test binaries and rustc run from or write into the build directory,
        so their command lines name it; cargo itself reads it from the
        environment and is not seen here.
        """
        if self.system == "Windows":
            script = "Get-CimInstance Win32_Process | ForEach-Object { $_.ExecutablePath; $_.CommandLine }"
            code, out = self.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", script])
        else:
            code, out = self.run(["ps", "-axo", "args="])
        if code != 0:
            raise PruneError("cannot list running processes: " + first_line(out))
        return [line for line in out.splitlines() if line.strip()]

    def remove_tree(self, path: Path) -> None:
        def clear_readonly(function, failed, _info):
            os.chmod(failed, stat.S_IWRITE)
            function(failed)

        shutil.rmtree(path, onerror=clear_readonly)


def dir_size(path: Path) -> int:
    total = 0
    stack = [str(path)]
    while stack:
        try:
            entries = list(os.scandir(stack.pop()))
        except OSError:
            continue
        for entry in entries:
            try:
                if entry.is_dir(follow_symlinks=False):
                    stack.append(entry.path)
                else:
                    total += entry.stat(follow_symlinks=False).st_size
            except OSError:
                continue
    return total


def last_change(path: Path) -> float:
    """Newest mtime of the directory and its two top levels.

    Cargo creates entries under `debug/` and `debug/deps` on every build, so
    two levels show the last build without walking the whole tree.
    """
    newest = 0.0
    level = [path]
    for _ in range(3):
        following = []
        for directory in level:
            try:
                newest = max(newest, directory.stat().st_mtime)
                for entry in os.scandir(directory):
                    newest = max(newest, entry.stat(follow_symlinks=False).st_mtime)
                    if entry.is_dir(follow_symlinks=False):
                        following.append(Path(entry.path))
            except OSError:
                continue
        level = following
    return newest


def first_line(text: str) -> str:
    return next((line.strip() for line in text.splitlines() if line.strip()), "")


def is_link(path: Path) -> bool:
    """Symlink or junction: deleting through it would hit another directory."""
    try:
        info = os.lstat(path)
    except OSError:
        return False
    return stat.S_ISLNK(info.st_mode) or bool(getattr(info, "st_file_attributes", 0) & REPARSE_POINT)


def normalized(path: Path, system: str) -> str:
    text = str(path).replace("\\", "/").rstrip("/")
    return text.lower() if system == "Windows" else text


def in_use(path: Path, command_lines: Sequence[str], system: str) -> bool:
    # `worker-1` must not match `worker-1-pair`, but must match
    # `CARGO_TARGET_DIR=C:/cargo-target/worker-1;` and `.../worker-1/debug`.
    pattern = re.compile(re.escape(normalized(path, system)) + r"(?![A-Za-z0-9_.-])")
    for line in command_lines:
        haystack = line.replace("\\", "/")
        if system == "Windows":
            haystack = haystack.lower()
        if pattern.search(haystack):
            return True
    return False


def default_build_root(system: str) -> Path:
    if system == "Windows":
        return Path(r"C:\cargo-target")
    return Path(os.environ.get("HOME", "~")) / "cargo-target"


def parse_worktrees(text: str) -> List[Worktree]:
    worktrees: List[Worktree] = []
    current: Dict[str, str] = {}
    for line in text.splitlines() + [""]:
        if not line.strip():
            if "worktree" in current:
                branch = current.get("branch", "")
                worktrees.append(Worktree(
                    Path(current["worktree"]),
                    branch[len("refs/heads/"):] if branch.startswith("refs/heads/") else None,
                    "locked" in current,
                ))
            current = {}
            continue
        key, _, value = line.partition(" ")
        current[key] = value
    return worktrees


class Pruner:
    def __init__(self, system: System, repo: Path, build_root: Path, stale_days: float) -> None:
        self.system = system
        self.repo = repo
        self.build_root = build_root
        self.stale = stale_days * DAY
        self.refresh()
        self.pr_cache: Dict[str, Optional[dict]] = {}

    def refresh(self) -> None:
        self.commands = self.system.process_command_lines()

    def quiet_for(self, path: Path, seconds: float) -> bool:
        return self.system.now() - last_change(path) > seconds

    def busy(self, path: Path) -> bool:
        return in_use(path, self.commands, self.system.system)

    def abandoned(self, path: Path) -> bool:
        return not self.busy(path) and self.quiet_for(path, self.stale)

    def find_pr(self, *selector: str) -> Optional[dict]:
        """The open PR the selector finds, else the most recent finished one."""
        key = " ".join(selector)
        if key not in self.pr_cache:
            code, out = self.system.run(
                ["gh", "pr", "list", *selector, "--state", "all", "--limit", "100",
                 "--json", "number,state,headRefOid"],
                cwd=self.repo,
            )
            if code != 0:
                raise PruneError(f"gh pr list {key} failed: {first_line(out)}")
            prs = json.loads(out) if out.strip() else []
            if selector[0] == "--search":
                # Text search also finds PRs that only mention the commit.
                prs = [pr for pr in prs if pr["headRefOid"] == selector[1]]
            prs.sort(key=lambda pr: (pr["state"] == "OPEN", pr["number"]), reverse=True)
            self.pr_cache[key] = prs[0] if prs else None
        return self.pr_cache[key]

    def pr_for_worktree(self, worktree: Worktree) -> Optional[dict]:
        """PR by branch name, else by the checked-out commit.

        A copy of someone else's PR often has a local branch name such as
        `pr-2265` or a detached HEAD; a PR whose head is exactly the
        checked-out commit is still found.
        """
        if worktree.branch:
            pr = self.find_pr("--head", worktree.branch)
            if pr:
                return pr
        code, head = self.git(worktree.path, "rev-parse", "HEAD")
        if code != 0 or not head.strip():
            return None
        return self.find_pr("--search", head.strip())

    def pr_state(self, number: int) -> Optional[str]:
        code, out = self.system.run(["gh", "pr", "view", str(number), "--json", "state"], cwd=self.repo)
        if code != 0:
            if "Could not resolve" in out or "no pull requests found" in out:
                return None
            raise PruneError(f"gh pr view {number} failed: {first_line(out)}")
        return json.loads(out).get("state")

    def git(self, worktree: Path, *args: str) -> Tuple[int, str]:
        return self.system.run(["git", "-C", str(worktree)] + list(args))

    def worktree_keep_reason(self, worktree: Worktree) -> Optional[str]:
        if worktree.locked:
            return "locked"
        code, out = self.git(worktree.path, "status", "--porcelain")
        if code != 0:
            return "git status failed"
        if out.strip():
            return "uncommitted changes"
        return None

    def build_dirs(self) -> List[Candidate]:
        found = []
        try:
            children = sorted(
                child for child in self.build_root.iterdir() if child.is_dir() and not is_link(child)
            )
        except OSError:
            return found
        for child in children:
            if self.busy(child):
                continue
            match = PR_SUFFIX.search(child.name)
            state = self.pr_state(int(match.group(1))) if match else None
            if state in FINISHED and self.quiet_for(child, GRACE_SECONDS):
                reason = f"PR #{match.group(1)} {state.lower()}"
            elif self.abandoned(child):
                reason = f"no process and unchanged for more than {self.stale / DAY:g} days"
            else:
                continue
            found.append(Candidate("build dir", str(child), reason, dir_size(child)))
        return found

    def worktrees(self) -> List[Candidate]:
        code, out = self.system.run(["git", "-C", str(self.repo), "worktree", "list", "--porcelain"])
        if code != 0:
            raise PruneError("git worktree list failed: " + first_line(out))
        listed = parse_worktrees(out)
        # The first entry is the main checkout; it is never removed.
        found = []
        here = normalized(self.repo.resolve(), self.system.system)
        for worktree in listed[1:]:
            path = worktree.path
            if normalized(path, self.system.system) == here or not path.is_dir() or is_link(path):
                continue
            reason = self.finished_worktree(worktree)
            pr = self.pr_for_worktree(worktree)
            open_pr = bool(pr and pr["state"] == "OPEN")
            if reason is None and not open_pr and self.abandoned(path):
                if self.worktree_keep_reason(worktree) is None and not self.has_unpushed(path):
                    reason = f"no open PR, no process and unchanged for more than {self.stale / DAY:g} days"
            if reason:
                found.append(Candidate("working copy", str(path), reason, dir_size(path)))
                continue
            target = path / "target"
            # A link to a build directory elsewhere is not a build in the copy.
            if (target.is_dir() and not is_link(target) and not self.busy(target)
                    and self.quiet_for(target, GRACE_SECONDS)):
                found.append(Candidate("in-copy target", str(target), "target/ inside a working copy", dir_size(target)))
        return found

    def finished_worktree(self, worktree: Worktree) -> Optional[str]:
        if self.busy(worktree.path):
            return None
        pr = self.pr_for_worktree(worktree)
        if not pr or pr["state"] not in FINISHED:
            return None
        if self.worktree_keep_reason(worktree):
            return None
        # Commits made after the PR's last head would be lost with the copy.
        code, _ = self.git(worktree.path, "merge-base", "--is-ancestor", "HEAD", pr["headRefOid"])
        if code != 0:
            return None
        return f"PR #{pr['number']} {pr['state'].lower()}"

    def has_unpushed(self, path: Path) -> bool:
        code, out = self.git(path, "rev-list", "--count", "HEAD", "--not", "--remotes")
        return code != 0 or out.strip() != "0"

    def candidates(self) -> List[Candidate]:
        return self.worktrees() + self.build_dirs()

    def remove(self, candidate: Candidate) -> Tuple[bool, str]:
        path = Path(candidate.path)
        if self.busy(path):
            return False, "in use"
        if is_link(path):
            return False, "is a link"
        if candidate.kind == "working copy":
            code, out = self.system.run(["git", "-C", str(self.repo), "worktree", "remove", str(path)])
            return code == 0, out.strip()
        try:
            self.system.remove_tree(path)
        except OSError as error:
            return False, str(error)
        return True, ""


def render(candidates: Sequence[Candidate], applied: Dict[str, Tuple[bool, str]], apply: bool) -> str:
    if not candidates:
        return "Nothing to prune."
    lines = []
    width = max(len(candidate.kind) for candidate in candidates)
    for candidate in candidates:
        line = f"{candidate.kind.ljust(width)}  {candidate.size / GIB:6.1f} GB  {candidate.path}  ({candidate.reason})"
        if candidate.path in applied:
            ok, message = applied[candidate.path]
            line += "  removed" if ok else f"  NOT removed: {message}"
        lines.append(line)
    total = sum(candidate.size for candidate in candidates)
    if apply:
        freed = sum(c.size for c in candidates if applied.get(c.path, (False, ""))[0])
        lines.append(f"Freed {freed / GIB:.1f} GB of {total / GIB:.1f} GB.")
    else:
        lines.append(f"{total / GIB:.1f} GB can be freed; run again with --apply to delete.")
    return "\n".join(lines)


def main(argv: Optional[Sequence[str]] = None, system: Optional[System] = None, repo: Path = REPO_ROOT) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--build-root", type=Path, help="directory holding the Cargo build directories")
    parser.add_argument("--stale-days", type=float, default=DEFAULT_STALE_DAYS,
                        help=f"age after which an unused directory counts as abandoned (default {DEFAULT_STALE_DAYS})")
    parser.add_argument("--apply", action="store_true", help="delete the candidates instead of only listing them")
    parser.add_argument("--json", action="store_true", help="print machine-readable results")
    args = parser.parse_args(argv)
    reconfigure = getattr(sys.stdout, "reconfigure", None)
    if reconfigure:
        reconfigure(errors="backslashreplace")
    system = system or System()
    try:
        pruner = Pruner(system, repo, args.build_root or default_build_root(system.system), args.stale_days)
        candidates = pruner.candidates()
        applied: Dict[str, Tuple[bool, str]] = {}
        if args.apply:
            # Sizing can take minutes; a build started meanwhile must survive.
            pruner.refresh()
            applied = {c.path: pruner.remove(c) for c in candidates}
    except PruneError as error:
        print(f"error: {error}; nothing was deleted", file=sys.stderr)
        return 2
    if args.json:
        print(json.dumps({
            "applied": args.apply,
            "candidates": [dict(asdict(c), removed=applied.get(c.path, (None,))[0]) for c in candidates],
        }, indent=2))
    else:
        print(render(candidates, applied, args.apply))
    return 0 if all(ok for ok, _ in applied.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
