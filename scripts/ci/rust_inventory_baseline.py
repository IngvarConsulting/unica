#!/usr/bin/env python3
"""Find the previous Rust inventory of this gate profile and its explanations.

The baseline is the `ci-rust-inventory-<profile>` artifact of the nearest
first-parent ancestor of HEAD. Queue runs test exactly the commit that becomes
`main`, so for a pull request or queue entry this is the last verified main.
Commit messages since that ancestor may explain an intended loss:
`Removes-test-target: <binary-id>`. In pull requests and queue entries only
the change under test itself may declare `Rust-inventory-baseline: reset`, so
a reset never covers the other entries of a queue batch; other profiles have
no batches and accept a reset already merged since their baseline. Only an empty search means "no baseline"; any API or
git failure stops.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ARTIFACT = "ci-rust-inventory-"
WORKFLOW = ".github/workflows/unica-plugin-release.yml"
ANCESTORS = 200
PAGES = 10
REMOVED = re.compile(r"^[ \t*-]*Removes-test-target:[ \t]*(\S+)[ \t]*$", re.MULTILINE)
RESET = re.compile(r"^[ \t*-]*Rust-inventory-baseline:[ \t]*reset[ \t]*$", re.MULTILINE)


def git(root: Path, *args: str) -> str:
    return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True).stdout


def gh(*args: str) -> str:
    completed = subprocess.run(["gh", *args], capture_output=True, text=True)
    if completed.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args[:2])} failed: {completed.stderr.strip()}")
    return completed.stdout


def candidate_names(profile: str) -> list[list[str]]:
    """Groups in priority order; the nearest ancestor wins inside a group."""
    # Pull requests never publish a baseline: they compare with the verified
    # target line, tested by the queue on main and by pushes on release lines.
    if profile == "pr":
        return [[ARTIFACT + "queue", ARTIFACT + "main"]]
    return [[ARTIFACT + name] for name in dict.fromkeys([profile, "queue"])]


def nearest(artifacts: list[dict], ancestors: list[str], repository_id: int) -> dict | None:
    rank = {sha: index for index, sha in enumerate(ancestors)}
    usable = [item for item in artifacts if not item.get("expired")
              and item.get("workflow_run", {}).get("head_sha") in rank
              and item["workflow_run"].get("repository_id") == repository_id
              and item["workflow_run"].get("head_repository_id") == repository_id]
    # Artifacts arrive newest first; min keeps the newest of the nearest commit.
    return min(usable, key=lambda item: rank[item["workflow_run"]["head_sha"]], default=None)


def find(repository: str, repository_id: int, profile: str, ancestors: list[str]) -> dict | None:
    for names in candidate_names(profile):
        artifacts = []
        for name in names:
            for page in range(1, PAGES + 1):
                listed = json.loads(gh("api", f"repos/{repository}/actions/artifacts?name={name}&per_page=100&page={page}"))
                artifacts += listed["artifacts"]
                if len(listed["artifacts"]) < 100:
                    break
        found = nearest(artifacts, ancestors, repository_id)
        if found is not None:
            return found
    return None


def download(repository: str, artifact: dict) -> dict:
    with tempfile.TemporaryDirectory() as temporary:
        gh("run", "download", str(artifact["workflow_run"]["id"]), "--repo", repository,
           "--name", artifact["name"], "--dir", temporary)
        document = json.loads((Path(temporary) / "inventory.json").read_text(encoding="utf-8"))
    run = artifact["workflow_run"]
    if (document.get("sha") != run["head_sha"] or document.get("runId") != str(run["id"])
        or ARTIFACT + str(document.get("profile")) != artifact["name"]):
        raise ValueError("previous inventory does not match its artifact run")
    if gh("api", f"repos/{repository}/actions/runs/{run['id']}", "--jq", ".path").strip() != WORKFLOW:
        raise ValueError("previous inventory was published by another workflow")
    return document


def explanations(root: Path, base: str | None, profile: str) -> dict:
    # Removals since the baseline stay explained for later queue entries.
    # In pr/queue a reset belongs to the change under test only: the pull
    # request commits behind its merge commit, or the entry's squash commit.
    revisions = f"{base}..HEAD" if base else "HEAD^1..HEAD"
    messages = git(root, "log", "--format=%B", revisions)
    own = git(root, "log", "--format=%B", "HEAD^1..HEAD") if profile in ("pr", "queue") else messages
    return {"range": revisions, "removedTargets": sorted(set(REMOVED.findall(messages))),
            "reset": RESET.search(own) is not None}


def baseline(root: Path, repository: str, repository_id: int, profile: str) -> dict:
    ancestors = git(root, "rev-list", "--first-parent", f"--max-count={ANCESTORS}", "HEAD").split()
    artifact = find(repository, repository_id, profile, ancestors)
    if artifact is None:
        return {"schemaVersion": 1, "source": None, "searchedAncestors": len(ancestors),
                "inventory": None, "explanations": explanations(root, None, profile)}
    source = {"artifact": artifact["name"], "runId": artifact["workflow_run"]["id"],
              "headSha": artifact["workflow_run"]["head_sha"]}
    return {"schemaVersion": 1, "source": source, "searchedAncestors": len(ancestors),
            "inventory": download(repository, artifact), "explanations": explanations(root, source["headSha"], profile)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    document = baseline(args.repo_root, os.environ["GITHUB_REPOSITORY"], int(os.environ["GITHUB_REPOSITORY_ID"]), args.profile)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    visible = json.dumps({key: document[key] for key in ("source", "searchedAncestors", "explanations")}, indent=2)
    print(visible)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write("### Previous Rust inventory\n\n```json\n" + visible + "\n```\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
