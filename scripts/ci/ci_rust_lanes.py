#!/usr/bin/env python3
"""Build each Rust configuration once, run archive shards, verify their union."""
from __future__ import annotations

import argparse
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parent))
import allure_results
import prepared_binary
import ci_shadow_selector
from workflow_routing import ADMITTED

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("ci_lane_runner", Path(__file__).with_name("run-tests.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
LANES = ("ordinary", "receipt", "vendor")


def lanes(profile: str) -> tuple[str, ...]:
    return LANES if set(ADMITTED[profile]) & {"medium", "large"} else ("ordinary",)


def selection(lane: str) -> tuple[str, ...]:
    return {"ordinary": ("--workspace",), "receipt": runner.LEDGER_FEATURE_SELECTION,
            "vendor": runner.UREQ_RECYCLING_SELECTION}[lane]


def shards(profile: str, lane: str) -> int:
    # A small starting matrix. Vendor's four fault cases need no extra runner.
    return 2 if lane == "ordinary" and profile != "pr" else 1


def matrix(profile: str, runners: list[str], *, lane: str, run: bool) -> list[dict]:
    if lane not in lanes(profile):
        return []
    return [{"runner": host, "lane": lane, "shard": index, "shards": shards(profile, lane)}
            for host in runners for index in (range(1, shards(profile, lane) + 1) if run else [1])]


def filters(lane: str) -> list[str]:
    selected = selection(lane)
    return list(selected[selected.index("-E"):]) if "-E" in selected else []


def archive_command(lane: str, archive: Path, root: Path = ROOT, target: str | None = None) -> list[str]:
    selected = list(selection(lane))
    if "-E" in selected:
        selected = selected[:selected.index("-E")]
    if "--target-dir" in selected:
        index = selected.index("--target-dir")
        del selected[index:index + 2]
    # Archive filters are binary predicates; the test predicate is applied to
    # list/run, never silently discarded from the execution plan.
    return ["cargo", "nextest", "archive", *selected, *([] if "--locked" in selected else ["--locked"]),
            *(["--target", target] if target is not None else []),
            "--target-dir", str(root / "target"), "--archive-file", str(archive)]


def reuse_command(action: str, root: Path, archive: Path, lane: str, profile: str, partition: str | None = None, *, candidate: str | None = None) -> list[str]:
    workspace = root / "vendor/ureq" if lane == "vendor" else root
    selected_filters = filters(lane)
    if candidate is not None:
        if selected_filters:
            if len(selected_filters) != 2 or selected_filters[0] != "-E":
                raise ValueError("unexpected lane filter configuration")
            # Repeated nextest -E flags are a union. Shadow selection must
            # remain inside the lane, so pass one intersected expression.
            candidate = f"({selected_filters[1]}) & ({candidate})"
        selected_filters = ["-E", candidate]
    command = ["cargo", "nextest", action, "--archive-file", str(archive),
               "--extract-to", str(root), "--extract-overwrite", "--workspace-remap", str(workspace),
               "--profile", runner.nextest_profile(profile, selection(lane)), *selected_filters]
    if lane == "vendor":
        command += ["--config-file", str(workspace / ".config/nextest.toml")]
    if partition:
        command += ["--partition", partition]
    if action == "list":
        command += ["--run-ignored", "all", "--message-format", "json"]
    return command


def inventory(document: dict) -> list[dict]:
    cases = []
    for suite in document["rust-suites"].values():
        for name, case in suite["testcases"].items():
            if case.get("filter-match", {}).get("status", "matches") == "matches":
                cases.append({"binary": suite["binary-id"], "name": name, "ignored": bool(case.get("ignored"))})
    return sorted(cases, key=case_id)


def case_id(case: dict) -> str:
    return case["binary"] + "::" + case["name"]


def validate_partition(full: list[dict], parts: list[list[dict]]) -> None:
    expected = Counter(case_id(case) for case in full)
    actual = Counter(case_id(case) for part in parts for case in part)
    if not expected or any(count != 1 for count in expected.values()) or actual != expected:
        raise ValueError("shard inventories must cover the nonempty complete plan exactly once")
    if sorted((case_id(c), c["ignored"]) for c in full) != sorted((case_id(c), c["ignored"]) for p in parts for c in p):
        raise ValueError("shard ignored state differs from the complete plan")


def write(path: Path, document: dict | list) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def signature(directory: Path, *, profile: str, host: str, lane: str, line: str | None, sha: str) -> None:
    path = allure_results.write_run(directory, profile=profile, runner=host, ecosystem="rust", line=line, sha=sha)
    document = json.loads(path.read_text())
    document.update(inventoryScope="full-nightly-v1" if profile == "large" else "gate-profile",
                    configuration={"lane": lane, "nextestProfile": runner.nextest_profile(profile, selection(lane)),
                                   "selection": list(selection(lane))})
    write(path, document)


def timed(command: list[str], root: Path, *, capture: bool = False):
    print("+ " + " ".join(command), flush=True)
    start = time.monotonic()
    completed = subprocess.run(command, cwd=root, text=True, stdout=subprocess.PIPE if capture else None)
    return completed, time.monotonic() - start


def junit_outcomes(path: Path) -> list[dict]:
    return [{"id": suite.get("name", "") + "::" + case.get("name", ""),
             "status": "skipped" if case.find("skipped") is not None else "failed" if case.find("failure") is not None or case.find("error") is not None else "passed"}
            for suite in ET.parse(path).getroot().iter("testsuite") for case in suite.findall("testcase")]


def build(root: Path, destination: Path, lane: str, profile: str, host: str, line: str | None) -> None:
    if destination.exists():
        raise ValueError(f"lane destination already exists: {destination}")
    destination.mkdir(parents=True)
    before = prepared_binary.identity(root)
    archive = destination / "tests.tar.zst"
    completed, seconds = timed(archive_command(lane, archive, root, before["target"]), root)
    completed.check_returncode()
    if prepared_binary.identity(root) != before:
        raise ValueError("source inputs changed while building the test archive")
    full_command = reuse_command("list", root, archive, lane, profile)
    completed, list_seconds = timed(full_command, root, capture=True)
    completed.check_returncode()
    full = inventory(json.loads(completed.stdout))
    parts = []
    for index in range(1, shards(profile, lane) + 1):
        command = reuse_command("list", root, archive, lane, profile, f"hash:{index}/{shards(profile, lane)}")
        completed, _ = timed(command, root, capture=True)
        completed.check_returncode()
        parts.append(inventory(json.loads(completed.stdout)))
    validate_partition(full, parts)
    document = {
        "schemaVersion": 1, "identity": before, "workspace": str(root.resolve()),
        "runner": host, "profile": profile, "lane": lane,
        "nextestProfile": runner.nextest_profile(profile, selection(lane)),
        "nextestVersion": prepared_binary.output(root, ["cargo", "nextest", "--version"]),
        "selection": list(selection(lane)), "archiveSha256": prepared_binary.digest(archive),
        "archiveBytes": archive.stat().st_size, "inventory": full, "shards": parts,
        "buildSeconds": seconds, "listSeconds": list_seconds,
        "runId": os.environ.get("GITHUB_RUN_ID"), "attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
    }
    write(destination / "lane.json", document)
    allure_results.write_plan(destination, full)
    signature(destination, profile=profile, host=host, lane=lane, line=line, sha=before["sha"])


def validate_manifest(document: dict, *, profile: str, host: str, lane: str, sha: str) -> None:
    if document.get("schemaVersion") != 1 or (document.get("profile"), document.get("runner"), document.get("lane"), document.get("identity", {}).get("sha")) != (profile, host, lane, sha):
        raise ValueError("archive or result belongs to another revision, runner, lane or profile")
    if document.get("selection") != list(selection(lane)) or document.get("nextestProfile") != runner.nextest_profile(profile, selection(lane)):
        raise ValueError("archive has an unexpected feature or test selection")
    if len(document.get("shards", [])) != shards(profile, lane):
        raise ValueError("archive has an unexpected shard count")
    validate_partition(document["inventory"], document["shards"])


def run_archive(root: Path, prepared: Path, destination: Path, lane: str, profile: str, host: str, index: int, line: str | None, shadow: Path | None = None) -> int:
    document = json.loads((prepared / "lane.json").read_text())
    current = prepared_binary.identity(root)
    validate_manifest(document, profile=profile, host=host, lane=lane, sha=current["sha"])
    if document["identity"] != current or document["workspace"] != str(root.resolve()):
        raise ValueError("archive has incompatible inputs or compiled absolute workspace paths")
    if document["nextestVersion"] != prepared_binary.output(root, ["cargo", "nextest", "--version"]):
        raise ValueError("archive requires its producer's exact nextest version")
    archive = prepared / "tests.tar.zst"
    if prepared_binary.digest(archive) != document["archiveSha256"] or archive.stat().st_size != document["archiveBytes"]:
        raise ValueError("test archive hash or size mismatch")
    if not 1 <= index <= len(document["shards"]):
        raise ValueError("invalid shard index")
    plan = document["shards"][index - 1]
    destination.mkdir(parents=True, exist_ok=False)
    allure_results.write_plan(destination, plan)
    signature(destination, profile=profile, host=host, lane=lane, line=line, sha=current["sha"])
    nextest_profile = document["nextestProfile"]
    candidates = {root / "target/nextest" / nextest_profile / "junit.xml", root / "vendor/ureq/target/nextest" / nextest_profile / "junit.xml"}
    for path in candidates:
        path.unlink(missing_ok=True)
    command = reuse_command("run", root, archive, lane, profile, f"hash:{index}/{len(document['shards'])}")
    if not any(not case["ignored"] for case in plan):
        command.append("--no-tests=pass")
    completed, seconds = timed(command, root)
    found = [path for path in candidates if path.is_file()]
    records = []
    if len(found) == 1:
        runner.emit_rust(destination, profile, host, found[0], selection=selection(lane))
        records = junit_outcomes(found[0])
    status = {
        "schemaVersion": 1, "sha": current["sha"], "profile": profile, "runner": host,
        "lane": lane, "shard": index, "archiveSha256": document["archiveSha256"],
        "plan": plan, "records": records, "exitCode": completed.returncode,
        "executionSeconds": seconds, "runId": os.environ.get("GITHUB_RUN_ID"),
        "attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
    }
    write(destination / "shard.json", status)
    validate_result(document, status, index)
    if shadow is not None:
        decision = json.loads(shadow.read_text())
        if decision.get("headSha") != current["sha"]:
            raise ValueError("shadow selection belongs to another revision")
        expression = ci_shadow_selector.filterset(decision)
        comparison = {"decision": decision, "fullTests": len(plan), "fullSeconds": seconds,
                      "kind": "full-fallback-reuses-full", "affectedTests": len(plan),
                      "selected": len(plan), "shadowNotRun": True}
        if expression is not None:
            available_packages = {case["binary"].split("::", 1)[0] for case in document["inventory"]}
            selected_packages = sorted(available_packages.intersection(decision["packages"]))
            comparison.update(archivePackages=sorted(available_packages), selectedPackages=selected_packages)
            if not selected_packages:
                comparison.update(kind="empty-lane-candidate", selected=0, affectedTests=0,
                                  affectedPlan=[], affectedSeconds=0, exitCode=0,
                                  reason="candidate packages have no tests in this archive lane")
                write(destination / "shadow.json", comparison)
                return completed.returncode
            expression = ci_shadow_selector.filterset({**decision, "packages": selected_packages})
            listing = reuse_command("list", root, archive, lane, profile, f"hash:{index}/{len(document['shards'])}", candidate=expression)
            listed, _ = timed(listing, root, capture=True)
            listed.check_returncode()
            affected = inventory(json.loads(listed.stdout))
            expected_affected = [case for case in plan if case["binary"].split("::", 1)[0] in selected_packages]
            if affected != expected_affected:
                raise ValueError("shadow selector and archive package inventory disagree")
            comparison.update(kind="affected-executed", selected=len(affected), affectedTests=len(affected), affectedPlan=affected)
            if any(not case["ignored"] for case in affected):
                for path in candidates:
                    path.unlink(missing_ok=True)
                shadow_command = reuse_command("run", root, archive, lane, profile, f"hash:{index}/{len(document['shards'])}", candidate=expression)
                shadow_run, shadow_seconds = timed(shadow_command, root)
                comparison.update(shadowNotRun=False, affectedSeconds=shadow_seconds, exitCode=shadow_run.returncode)
                shadow_reports = [path for path in candidates if path.is_file()]
                if shadow_run.returncode != 0 or len(shadow_reports) != 1:
                    write(destination / "shadow.json", comparison)
                    raise ValueError("shadow execution failed or did not report results")
                comparison["records"] = junit_outcomes(shadow_reports[0])
                write(destination / "shadow.json", comparison)
                shadow_manifest = {**document, "shards": list(document["shards"])}
                shadow_manifest["shards"][index - 1] = affected
                validate_result(shadow_manifest, {**status, "plan": affected, "records": comparison["records"], "exitCode": shadow_run.returncode}, index)
            else:
                comparison.update(kind="affected-no-runnable-tests", affectedSeconds=0, exitCode=0)
        write(destination / "shadow.json", comparison)
    return completed.returncode


def validate_result(manifest: dict, result: dict, index: int) -> None:
    expected = (manifest["identity"]["sha"], manifest["profile"], manifest["runner"], manifest["lane"], index, manifest["archiveSha256"])
    actual = tuple(result.get(key) for key in ("sha", "profile", "runner", "lane", "shard", "archiveSha256"))
    if result.get("schemaVersion") != 1 or actual != expected or result.get("plan") != manifest["shards"][index - 1]:
        raise ValueError("missing or mismatched shard identity or plan")
    if any(result.get(key) != manifest.get(key) for key in ("runId", "attempt")):
        raise ValueError("shard result belongs to another workflow run or attempt")
    wanted = {case_id(case) for case in result["plan"] if not case["ignored"]}
    allowed = {case_id(case) for case in result["plan"]}
    records = result.get("records", [])
    observed = Counter(record["id"] for record in records)
    if any(count != 1 for count in observed.values()) or not wanted.issubset(observed) or not set(observed).issubset(allowed):
        raise ValueError("shard results do not cover their assigned tests exactly once")
    if any(record["status"] == "skipped" and record["id"] in wanted for record in records):
        raise ValueError("a required shard test did not execute")
    if result.get("exitCode") != 0 or any(record["status"] not in ("passed", "skipped") for record in records):
        raise ValueError("a required shard test failed")


def verify_union(directory: Path, profile: str, hosts: list[str], sha: str) -> dict:
    if not hosts or len(hosts) != len(set(hosts)) or any(host not in {"ubuntu-latest", "macos-14", "windows-latest"} for host in hosts):
        raise ValueError("missing, duplicate or unknown OS inventory")
    manifests = [json.loads(path.read_text()) for path in directory.rglob("lane.json")]
    results = [json.loads(path.read_text()) for path in directory.rglob("shard.json")]
    problems, advisory = [], []
    if os.environ.get("CI_LANE_JOB_RESULTS"):
        expected_hosts = ["ubuntu-latest", "macos-14", "windows-latest"] if profile == "large" else ["ubuntu-latest", "macos-14"]
        if hosts != expected_hosts and not (profile == "pr" and hosts == ["ubuntu-latest"]):
            problems.append("runner matrix omits an expected operating system")
        jobs = json.loads(os.environ["CI_LANE_JOB_RESULTS"])
        for lane in LANES:
            expected = "success" if lane in lanes(profile) else "skipped"
            for phase in ("build", "run"):
                job = f"{phase}-rust-{lane}"
                if jobs.get(job, {}).get("result") != expected:
                    problems.append(f"{job} did not finish as {expected}")
        if jobs.get("shadow-component-selection", {}).get("result") != "success":
            problems.append("shadow selection did not finish successfully")
    expected_configs = {(host, lane) for host in hosts for lane in lanes(profile)}
    if any((item.get("runner"), item.get("lane")) not in expected_configs for item in manifests + results):
        raise ValueError("unexpected runner or lane in the collected inventory")
    for host, lane in sorted(expected_configs):
        try:
            matching = [item for item in manifests if (item.get("runner"), item.get("lane")) == (host, lane)]
            if len(matching) != 1:
                raise ValueError("missing or duplicate lane plan")
            manifest = matching[0]
            validate_manifest(manifest, profile=profile, host=host, lane=lane, sha=sha)
            if os.environ.get("GITHUB_RUN_ID") and any(manifest.get(key) != os.environ.get(env) for key, env in (("runId", "GITHUB_RUN_ID"), ("attempt", "GITHUB_RUN_ATTEMPT"))):
                raise ValueError("lane plan belongs to another workflow run or attempt")
            matching_results = [item for item in results if (item.get("runner"), item.get("lane")) == (host, lane)]
            if len(matching_results) != len(manifest["shards"]):
                raise ValueError("missing or duplicate shard result")
            if sorted(item.get("shard", 0) for item in matching_results) != list(range(1, len(manifest["shards"]) + 1)):
                raise ValueError("missing or duplicate shard index")
            for result in matching_results:
                validate_result(manifest, result, result["shard"])
        except (KeyError, TypeError, ValueError) as error:
            target = advisory if profile == "large" and host == "windows-latest" else problems
            target.append(f"{host}/{lane}: {error}")
    report = {"sha": sha, "profile": profile, "mandatoryComplete": not problems,
              "fullMatrixPassed": not problems and not advisory, "errors": problems,
              "advisoryFailures": advisory, "affectedAdmission": False}
    write(directory / "verification.json", report)
    if problems:
        raise ValueError("; ".join(problems))
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("matrix", "build", "run", "verify"))
    parser.add_argument("--repo-root", type=Path, default=ROOT)
    parser.add_argument("--profile", required=True, choices=tuple(ADMITTED))
    parser.add_argument("--lane", choices=LANES)
    parser.add_argument("--runner")
    parser.add_argument("--runners", default="[]")
    parser.add_argument("--destination", type=Path)
    parser.add_argument("--prepared", type=Path)
    parser.add_argument("--shard", type=int, default=1)
    parser.add_argument("--run-matrix", action="store_true")
    parser.add_argument("--sha", default=os.environ.get("GITHUB_SHA"))
    parser.add_argument("--line")
    parser.add_argument("--shadow", type=Path)
    args = parser.parse_args()
    if args.action == "matrix":
        print(json.dumps(matrix(args.profile, json.loads(args.runners), lane=args.lane, run=args.run_matrix), separators=(",", ":")))
    elif args.action == "verify":
        print(json.dumps(verify_union(args.destination, args.profile, json.loads(args.runners), args.sha), indent=2))
    elif args.action == "build":
        build(args.repo_root.resolve(), args.destination, args.lane, args.profile, args.runner, args.line)
    else:
        return run_archive(args.repo_root.resolve(), args.prepared, args.destination, args.lane, args.profile, args.runner, args.shard, args.line, args.shadow)
    return 0


if __name__ == "__main__":
    sys.exit(main())
