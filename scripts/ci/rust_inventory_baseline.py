#!/usr/bin/env python3
"""Find the previous Rust inventory of this gate profile and its explanations.

The baseline is the `ci-rust-inventory-<profile>` artifact of the nearest
first-parent ancestor of HEAD. Queue runs test exactly the commit that becomes
`main`, so for a pull request or queue entry this is the last verified main.
An intended loss is explained in the description of a pull request merged
since that ancestor, where reviewers see it: `Removes-test-target: <binary-id>`
or `Removes-test: <binary-id>::<test name, or prefix ending in *>`. In pull
requests and queue entries only the change under test itself may declare
`Rust-inventory-baseline: reset`, so a reset never covers the other entries of
a queue batch; other profiles have no batches and accept a reset already
merged since their baseline. Only an empty search means "no baseline"; any API
or git failure stops.
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
REMOVED = re.compile(r"^[ \t*-]*Removes-test-target:[ \t]*`?([^\s`]+)`?[ \t]*$", re.MULTILINE)
REMOVED_TEST = re.compile(r"^[ \t*-]*Removes-test:[ \t]*`?([^\s`]+)`?[ \t]*$", re.MULTILINE)
RESET = re.compile(r"^[ \t*-]*`?Rust-inventory-baseline:[ \t]*reset`?[ \t]*$", re.MULTILINE)
# Squash and merge subjects name their pull request; issue references inside
# a title precede the final `(#N)` and are not taken.
SQUASHED = re.compile(r"\(#(\d+)\)\s*$")
MERGED = re.compile(r"^Merge pull request #(\d+) ")


def git(root: Path, *args: str) -> str:
    return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True).stdout


class NotFound(RuntimeError):
    pass


def gh(*args: str) -> str:
    completed = subprocess.run(["gh", *args], capture_output=True, text=True)
    if completed.returncode != 0:
        error = NotFound if "HTTP 404" in completed.stderr else RuntimeError
        raise error(f"gh {' '.join(args[:2])} failed: {completed.stderr.strip()}")
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


def pull_requests(root: Path, revisions: str) -> set[int]:
    numbers = set()
    # First parents only: a pull request branch may carry commits whose
    # subjects name other, unmerged pull requests.
    for subject in git(root, "log", "--first-parent", "--format=%s", revisions).splitlines():
        found = SQUASHED.search(subject) or MERGED.match(subject)
        if found:
            numbers.add(int(found.group(1)))
    return numbers


def description(repository: str, number: int) -> str | None:
    try:
        # Descriptions edited on the web often end lines with CRLF.
        return gh("api", f"repos/{repository}/pulls/{number}", "--jq", ".body // \"\"").replace("\r\n", "\n")
    except NotFound:
        return None  # `(#N)` named an issue, not a pull request


def explanations(root: Path, repository: str, base: str | None, profile: str, own: int | None) -> dict:
    # Removals since the baseline stay explained for later queue entries.
    # In pr/queue a reset belongs to the change under test only: the pull
    # request itself, or the pull request of the entry's squash commit.
    revisions = f"{base}..HEAD" if base else "HEAD^1..HEAD"
    merged = pull_requests(root, revisions) | ({own} if own else set())
    if profile == "pr":
        resetting = {own} if own else set()
    elif profile == "queue":
        resetting = pull_requests(root, "HEAD^1..HEAD")
    else:
        resetting = merged
    declared: dict[str, list[int]] = {}
    targets, tests, reset = set(), set(), set()
    for number in sorted(merged | resetting):
        body = description(repository, number)
        if body is None:
            continue
        for kind, pattern, items in (("Removes-test-target", REMOVED, targets), ("Removes-test", REMOVED_TEST, tests)):
            for item in pattern.findall(body):
                items.add(item)
                declared.setdefault(f"{kind}: {item}", []).append(number)
        if number in resetting and RESET.search(body):
            reset.add(number)
    if reset:
        declared["Rust-inventory-baseline: reset"] = sorted(reset)
    return {"range": revisions, "pullRequests": sorted(merged), "removedTargets": sorted(targets),
            "removedTests": sorted(tests), "reset": bool(reset), "declaredBy": declared}


def own_pull_request() -> int | None:
    path = os.environ.get("GITHUB_EVENT_PATH")
    if os.environ.get("GITHUB_EVENT_NAME") != "pull_request" or not path:
        return None
    return int(json.loads(Path(path).read_text(encoding="utf-8"))["pull_request"]["number"])


def baseline(root: Path, repository: str, repository_id: int, profile: str, own: int | None = None) -> dict:
    ancestors = git(root, "rev-list", "--first-parent", f"--max-count={ANCESTORS}", "HEAD").split()
    artifact = find(repository, repository_id, profile, ancestors)
    if artifact is None:
        return {"schemaVersion": 1, "source": None, "searchedAncestors": len(ancestors),
                "inventory": None, "explanations": explanations(root, repository, None, profile, own)}
    source = {"artifact": artifact["name"], "runId": artifact["workflow_run"]["id"],
              "headSha": artifact["workflow_run"]["head_sha"]}
    return {"schemaVersion": 1, "source": source, "searchedAncestors": len(ancestors),
            "inventory": download(repository, artifact),
            "explanations": explanations(root, repository, source["headSha"], profile, own)}


def summary_text(document: dict) -> str:
    """Name each accepted declaration and its pull requests for reviewers."""
    source = document["source"]
    lines = ["### Previous Rust inventory", "",
             f"Baseline: run {source['runId']} at `{source['headSha']}`" if source else "Baseline: **none found**", ""]
    declared = document["explanations"]["declaredBy"]
    if "Rust-inventory-baseline: reset" in declared:
        lines += ["**Baseline reset declared by " + ", ".join(f"#{n}" for n in declared["Rust-inventory-baseline: reset"])
                  + ": inventory losses are not blocking.**", ""]
    lines += [f"- `{item}` — " + ", ".join(f"#{n}" for n in numbers) for item, numbers in sorted(declared.items())
              if item != "Rust-inventory-baseline: reset"] or ["No removals declared."]
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    document = baseline(args.repo_root, os.environ["GITHUB_REPOSITORY"], int(os.environ["GITHUB_REPOSITORY_ID"]),
                        args.profile, own_pull_request())
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    visible = json.dumps({key: document[key] for key in ("source", "searchedAncestors", "explanations")}, indent=2)
    print(visible)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write(summary_text(document) + "\n```json\n" + visible + "\n```\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
