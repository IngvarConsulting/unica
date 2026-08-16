#!/usr/bin/env python3
"""Advance or report a release that is part-way through publication.

The release is a two-phase publication, and every phase boundary used to be a
manual step. Those steps are mechanical, but the waiting between them is not
bounded: the marketplace tag is a human decision that may arrive hours later. A
job that waited would either time out or burn minutes, so this runs on a
schedule instead and moves the release one step whenever it can.

The state machine is deliberately small:

    catalog ref == latest release      -> nothing in flight
    staging pull request, all green    -> merge it, then ask for promotion
    promotion red, tag now exists      -> rerun the failed checks, once
    promotion pull request, all green  -> merge it, the release goes live
    anything else                      -> report what is being waited on

Green is the gate everywhere. A promotion pull request cannot be green until the
marketplace tag exists, because its checks install through the catalog ref, so
merging on green keeps the tag as the human approval rather than bypassing it.
The marketplace default branch has no protection rules, so the greenness check
here is the only gate and must never be relaxed.

The rerun exists because a tag push re-triggers nothing on the pull request:
the checks that failed before the tag stay failed on their own, and the release
would stall right after the human's approval. One rerun is enough — a failure
on a later attempt cannot be explained by the missing tag, so it is an alert,
not another retry.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from typing import Sequence


MARKETPLACE = "IngvarConsulting/unica-marketplace"
SOURCE = "IngvarConsulting/unica"
CATALOG_PATHS = {
    ".agents/plugins/marketplace.json",
    ".claude-plugin/marketplace.json",
}
STAGE_BRANCH = re.compile(r"^codex/stage-(v\d+\.\d+\.\d+)-(\d+)$")
PROMOTE_BRANCH = re.compile(r"^codex/promote-(v\d+\.\d+\.\d+)-(\d+)$")
PENDING_STATES = {"PENDING", "QUEUED", "IN_PROGRESS", "WAITING", "REQUESTED", None, ""}
PASSING_CONCLUSIONS = {"SUCCESS", "NEUTRAL", "SKIPPED"}


@dataclass(frozen=True)
class PullRequest:
    number: int
    branch: str
    files: tuple[str, ...]
    checks: tuple[tuple[str, str | None, str | None], ...]  # name, status, conclusion

    @property
    def release_tag(self) -> str | None:
        match = STAGE_BRANCH.match(self.branch) or PROMOTE_BRANCH.match(self.branch)
        return match.group(1) if match else None

    @property
    def source_run_id(self) -> str | None:
        match = STAGE_BRANCH.match(self.branch) or PROMOTE_BRANCH.match(self.branch)
        return match.group(2) if match else None

    def blocking_checks(self) -> list[str]:
        blocking = []
        for name, status, conclusion in self.checks:
            if status is not None and status.upper() in PENDING_STATES and conclusion is None:
                blocking.append(f"{name}: pending")
            elif conclusion is not None and conclusion.upper() not in PASSING_CONCLUSIONS:
                blocking.append(f"{name}: {conclusion.lower()}")
        return blocking

    def is_green(self) -> bool:
        return bool(self.checks) and not self.blocking_checks()


@dataclass(frozen=True)
class WorkflowRun:
    id: int
    attempt: int


@dataclass(frozen=True)
class ReleaseState:
    latest_release: str | None
    catalog_ref: str | None
    stage_pr: PullRequest | None = None
    promote_pr: PullRequest | None = None
    marketplace_tag_exists: bool = False
    promotion_checks_run: WorkflowRun | None = None


@dataclass(frozen=True)
class Action:
    kind: str  # noop | merge-stage | merge-promote | kick-promotion | wait | alert
    detail: str
    pull_request: int | None = None
    context: dict[str, str] = field(default_factory=dict)


def decide(state: ReleaseState) -> Action:
    if state.latest_release is None:
        return Action("alert", "no published source release found")
    if state.catalog_ref == state.latest_release:
        return Action("noop", f"catalog already serves {state.latest_release}")

    detail_suffix = f"catalog serves {state.catalog_ref}, latest release is {state.latest_release}"

    stage = state.stage_pr
    if stage is not None:
        if not stage.is_green():
            return Action(
                "wait",
                f"staging checks are not green: {', '.join(stage.blocking_checks())}",
                stage.number,
            )
        return Action(
            "merge-stage",
            f"staging is green, merging so the payload lands; {detail_suffix}",
            stage.number,
            {"release_tag": stage.release_tag or "", "source_run_id": stage.source_run_id or ""},
        )

    promote = state.promote_pr
    if promote is not None:
        unexpected = sorted(set(promote.files) - CATALOG_PATHS)
        if unexpected:
            return Action(
                "alert",
                f"promotion pull request changes more than the catalog: {', '.join(unexpected)}",
                promote.number,
            )
        blocking = promote.blocking_checks()
        if blocking:
            # The install checks resolve the catalog ref, so they cannot pass
            # before the marketplace tag is published. That is the human gate.
            pending = any(item.endswith(": pending") for item in blocking)
            if state.marketplace_tag_exists and not pending:
                # A tag push re-triggers nothing on the pull request, so a
                # failure observed before the tag existed stays red on its own.
                # The first rerun is the warden's to make; a failure on a later
                # attempt cannot be explained by the missing tag and retrying
                # it would only hide a broken install path.
                run = state.promotion_checks_run
                if run is not None and run.attempt == 1:
                    return Action(
                        "kick-promotion",
                        f"the {promote.release_tag} tag exists, rerunning the failed "
                        f"promotion checks: {', '.join(blocking)}",
                        promote.number,
                        {"run_id": str(run.id)},
                    )
                return Action(
                    "alert",
                    f"promotion checks failed although the {promote.release_tag} tag "
                    f"exists: {', '.join(blocking)}",
                    promote.number,
                )
            return Action(
                "wait",
                f"promotion is waiting for the {promote.release_tag} tag or a green run: "
                f"{', '.join(blocking)}",
                promote.number,
            )
        return Action(
            "merge-promote",
            f"promotion is green, publishing {promote.release_tag}",
            promote.number,
        )

    return Action("alert", f"release is stalled with no open pull request; {detail_suffix}")


def latest_servable_release(releases: Sequence[dict]) -> str | None:
    """The newest release that is meant to reach consumers.

    Abandoning a version is a legitimate outcome: once its assets are published
    the number is burnt and the next patch is cut instead. Without a way to say
    so, the warden would compare the catalog against a version nobody intends to
    serve and alert forever. Marking the release as a draft or prerelease is that
    signal, and it uses a field GitHub already has rather than a marker file.
    """
    for release in releases:
        if release.get("isDraft") or release.get("isPrerelease"):
            continue
        return release.get("tagName")
    return None


def gh_json(args: Sequence[str]) -> object:
    result = subprocess.run(
        ["gh", *args], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
    )
    return json.loads(result.stdout or "null")


def load_pull_request(entry: dict) -> PullRequest:
    checks = tuple(
        (
            item.get("name") or item.get("context") or "check",
            item.get("status"),
            item.get("conclusion") or item.get("state"),
        )
        for item in entry.get("statusCheckRollup") or []
    )
    return PullRequest(
        number=entry["number"],
        branch=entry["headRefName"],
        files=tuple(item["path"] for item in entry.get("files") or []),
        checks=checks,
    )


def observe() -> ReleaseState:
    releases = gh_json(
        [
            "release",
            "list",
            "--repo",
            SOURCE,
            "--limit",
            "10",
            "--json",
            "tagName,isDraft,isPrerelease",
        ]
    )
    latest = latest_servable_release(releases or [])

    catalog = subprocess.run(
        [
            "gh",
            "api",
            f"repos/{MARKETPLACE}/contents/.agents/plugins/marketplace.json",
            "--jq",
            ".content",
        ],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout
    import base64

    entry = json.loads(base64.b64decode(catalog))["plugins"][0]
    catalog_ref = entry.get("source", {}).get("ref")

    open_prs = gh_json(
        [
            "pr",
            "list",
            "--repo",
            MARKETPLACE,
            "--state",
            "open",
            "--json",
            "number,headRefName,files,statusCheckRollup",
        ]
    )
    stage_pr = None
    promote_pr = None
    for item in open_prs or []:
        pull = load_pull_request(item)
        if STAGE_BRANCH.match(pull.branch):
            stage_pr = pull
        elif PROMOTE_BRANCH.match(pull.branch):
            promote_pr = pull

    marketplace_tag_exists = False
    promotion_checks_run = None
    if promote_pr is not None and promote_pr.release_tag:
        probe = subprocess.run(
            [
                "gh",
                "api",
                f"repos/{MARKETPLACE}/git/ref/tags/{promote_pr.release_tag}",
                "--silent",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        marketplace_tag_exists = probe.returncode == 0
        runs = gh_json(
            [
                "run",
                "list",
                "--repo",
                MARKETPLACE,
                "--branch",
                promote_pr.branch,
                "--limit",
                "1",
                "--json",
                "databaseId,attempt",
            ]
        )
        if runs:
            promotion_checks_run = WorkflowRun(
                id=runs[0]["databaseId"], attempt=runs[0].get("attempt") or 1
            )

    return ReleaseState(
        latest,
        catalog_ref,
        stage_pr,
        promote_pr,
        marketplace_tag_exists=marketplace_tag_exists,
        promotion_checks_run=promotion_checks_run,
    )


def apply(action: Action, *, dry_run: bool) -> None:
    if dry_run or action.pull_request is None:
        return
    if action.kind == "merge-stage":
        subprocess.run(
            ["gh", "pr", "merge", str(action.pull_request), "--repo", MARKETPLACE, "--merge"],
            check=True,
        )
        merge_sha = gh_json(
            [
                "pr",
                "view",
                str(action.pull_request),
                "--repo",
                MARKETPLACE,
                "--json",
                "mergeCommit",
            ]
        )["mergeCommit"]["oid"]
        subprocess.run(
            [
                "gh",
                "workflow",
                "run",
                "publish-unica-marketplace.yml",
                "--repo",
                SOURCE,
                "-f",
                "mode=promote",
                "-f",
                f"source_run_id={action.context['source_run_id']}",
                "-f",
                f"release_tag={action.context['release_tag']}",
                "-f",
                f"staging_merge_sha={merge_sha}",
            ],
            check=True,
        )
    elif action.kind == "merge-promote":
        subprocess.run(
            ["gh", "pr", "merge", str(action.pull_request), "--repo", MARKETPLACE, "--merge"],
            check=True,
        )
    elif action.kind == "kick-promotion":
        subprocess.run(
            [
                "gh",
                "run",
                "rerun",
                action.context["run_id"],
                "--failed",
                "--repo",
                MARKETPLACE,
            ],
            check=True,
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--alert-is-failure",
        action="store_true",
        help="exit non-zero on alert, so a stalled release surfaces as a failed run",
    )
    args = parser.parse_args()

    action = decide(observe())
    print(f"{action.kind}: {action.detail}")
    apply(action, dry_run=args.dry_run)
    if action.kind == "alert" and args.alert_is_failure:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
