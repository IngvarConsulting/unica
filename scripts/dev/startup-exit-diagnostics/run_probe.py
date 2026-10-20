#!/usr/bin/env python3
"""Pinned Windows-only #1714 observation; never reruns a failing test."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

PREFIX = "infrastructure::daemon::client_v5::"
ORIGINAL = [PREFIX + "tests::exit_before_readiness_reports_only_the_childs_own_startup_failure"]
CONTROLS = [
    PREFIX + "tests::failed_readiness_kills_the_child_within_the_original_budget",
    PREFIX + "tests::startup_observation_confirmed_exit_preserves_pid_attribution",
    PREFIX + "tests::startup_observation_expired_readiness_still_precedes_confirmed_exit",
    PREFIX + "startup_observation::first_stop_and_observation_interval_survive_later_diagnostics",
]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def execution_outcomes(cases):
    """Only a normal Rust test failure permits the subsequent observation."""
    outcomes = []
    for case in cases:
        failures = list(case.findall("failure")) + list(case.findall("error"))
        outcome = {"name": case.attrib["name"], "causes": [
            {"element": node.tag, "type": node.get("type"), "message": node.get("message")}
            for node in failures
        ]}
        outcome["permitsNextObservation"] = not failures or (
            len(failures) == 1 and failures[0].tag == "failure"
            and failures[0].get("type") == "test failure with exit code 101"
        )
        outcomes.append(outcome)
    return outcomes


def run(argv, cwd, output, label, env, timeout, check=True):
    item = {"argv": list(map(str, argv)), "cwd": str(cwd), "startedUnixNs": time.time_ns(),
            "timeoutSeconds": timeout, "state": "started"}
    prefix = output / label
    write(prefix.with_suffix(".json"), item)
    try:
        with prefix.with_suffix(".stdout").open("wb") as stdout, prefix.with_suffix(".stderr").open("wb") as stderr:
            completed = subprocess.run(item["argv"], cwd=cwd, env=env, stdout=stdout, stderr=stderr, timeout=timeout)
        item.update(state="completed", exit=completed.returncode)
    except subprocess.TimeoutExpired:
        # subprocess stops only its direct child. This is invalid evidence,
        # never successful cleanup; the disposable Actions job bounds leftovers.
        item.update(state="timed_out", exit=None)
    finally:
        item.update(finishedUnixNs=time.time_ns())
        for stream in ["stdout", "stderr"]:
            if prefix.with_suffix("." + stream).exists():
                item[stream + "Sha256"] = sha(prefix.with_suffix("." + stream))
        write(prefix.with_suffix(".json"), item)
    if check:
        require(item.get("exit") == 0, f"{label}: {item['state']}, exit={item.get('exit')}")
    return item


def execute(args, receipt):
    case, output = args.case.resolve(), args.results.resolve()
    payload = Path(__file__).resolve().parent
    manifest_path = payload / "manifest.json"
    require(sha(manifest_path) == args.manifest_sha256, "control manifest hash differs")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    require(re.fullmatch(r"[0-9a-f]{40}", manifest["caseCommit"]), "case commit placeholder must be frozen by root")
    require(args.case_commit == manifest["caseCommit"], "unreviewed case commit")
    require(sha(__file__) == manifest["runnerSha256"], "control runner hash differs")
    require(platform.system() == "Windows" and platform.machine().lower() in {"amd64", "x86_64"}, "native Windows x64 required")
    require(os.environ.get("GITHUB_RUN_ATTEMPT") == "1", "preserve the first attempt; do not rerun this diagnostic")
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=case, text=True).strip() == args.case_commit, "case checkout differs")
    control = payload.parents[2]
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=control, text=True).strip() == os.environ["GITHUB_SHA"], "control checkout differs")

    def verify_source():
        require(not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=case), "tracked case source changed")
        for path, expected in manifest["sourcePins"].items():
            require(sha(case / path) == expected, "case pin differs: " + path)

    verify_source()
    receipt.update(caseCommit=args.case_commit, controlCommit=os.environ["GITHUB_SHA"], manifestSha256=sha(manifest_path),
                   os=platform.platform(), machine=platform.machine(), sourcePins=manifest["sourcePins"],
                   provenance={k: os.environ.get(k) for k in ["GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "GITHUB_REF", "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion"]})
    env = dict(os.environ)
    for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTC_BOOTSTRAP", "CARGO_BUILD_TARGET", "UNICA_STARTUP_EXIT_FIXTURE", "NEXTEST_PROFILE", "NEXTEST_RETRIES", "NEXTEST_TEST_THREADS"]:
        env.pop(name, None)
    target = output / "target"
    require(not target.exists(), "target must be fresh and task-owned")
    env.update(CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="0", CARGO_TERM_COLOR="never")
    receipt["explicitEnvironment"] = {k: env[k] for k in ["CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "CARGO_TERM_COLOR"]}
    for label, argv in [("rustc-version", ["rustc", "-Vv"]), ("cargo-version", ["cargo", "-V"]), ("nextest-version", ["cargo", "nextest", "--version"])]:
        run(argv, case, output, label, env, 30)
    require("0.9.148" in (output / "nextest-version.stdout").read_text(), "nextest version differs")
    identity = "$items = @(Get-Command powershell.exe -CommandType Application -All | ForEach-Object { [pscustomobject]@{path=$_.Source; sha256=(Get-FileHash -LiteralPath $_.Source -Algorithm SHA256).Hash; version=(Get-Item -LiteralPath $_.Source).VersionInfo.FileVersion} }); [pscustomobject]@{candidates=$items; path=$env:PATH; parentPSVersion=$PSVersionTable.PSVersion.ToString()} | ConvertTo-Json -Depth 5"
    run(["pwsh", "-NoProfile", "-NonInteractive", "-Command", identity], case, output, "powershell-identity", env, 30)
    require(json.loads((output / "powershell-identity.stdout").read_text(encoding="utf-8-sig"))["candidates"], "PowerShell executable unresolved")
    run(["cargo", "test", "--locked", "-p", "unica-coder", "--lib", "--no-run", "--message-format=json"], case, output, "build", env, 2400)
    artifacts = []
    for line in (output / "build.stdout").read_text(encoding="utf-8").splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact" and item.get("profile", {}).get("test") and item.get("executable") and Path(item["target"]["src_path"]).resolve() == case / "crates/unica-coder/src/lib.rs":
            artifacts.append(item)
    require(len(artifacts) == 1 and artifacts[0]["fresh"] is False, "one fresh case-owned libtest artifact required")
    binary = Path(artifacts[0]["executable"]).resolve()
    require(binary.is_relative_to(target) and binary.suffix == ".exe", "artifact outside owned Windows target")
    binary_hash = sha(binary)
    retained = output / "retained-unica_coder-libtest.exe"
    shutil.copy2(binary, retained)
    receipt["artifact"] = {"compilerMessage": artifacts[0], "path": str(binary), "sha256": binary_hash, "retainedPath": str(retained)}
    write(output / "artifact.json", receipt["artifact"])

    # Both are pre-execution preparation. Official nextest binary-list generation
    # may call Cargo; fail if the already-retained compiler artifact changes.
    run(["cargo", "metadata", "--locked", "--format-version", "1"], case, output, "cargo-metadata", env, 120)
    run(["cargo", "nextest", "list", "--locked", "-p", "unica-coder", "--lib", "--list-type", "binaries-only", "--message-format", "json"], case, output, "binaries-metadata", env, 600)
    binaries = json.loads((output / "binaries-metadata.stdout").read_text())["rust-binaries"]
    require(len(binaries) == 1 and Path(next(iter(binaries.values()))["binary-path"]).resolve() == binary, "nextest artifact differs from Cargo artifact")
    reuse = ["--cargo-metadata", str(output / "cargo-metadata.stdout"), "--binaries-metadata", str(output / "binaries-metadata.stdout"), "--profile", "queue", "--user-config-file", "none"]
    results = []
    try:
        for label, names in [("original", ORIGINAL), ("controls", CONTROLS)]:
            verify_source()
            require(sha(binary) == binary_hash and sha(retained) == binary_hash, "retained artifact changed")
            selector = " | ".join("test(=" + name + ")" for name in names)
            run(["cargo", "nextest", "list", *reuse, "-E", selector, "--message-format", "json"], case, output, label + "-selection", env, 60)
            listed = json.loads((output / (label + "-selection.stdout")).read_text())
            selected = []
            for suite in listed["rust-suites"].values():
                require(Path(suite["binary-path"]).resolve() == binary, "listed test binary differs")
                selected.extend(name for name, entry in suite["testcases"].items() if not entry["ignored"] and entry["filter-match"]["status"] == "matches")
            require(sorted(selected) == sorted(names), "exact selection is missing, duplicated or includes another test")
            result = run(["cargo", "nextest", "run", *reuse, "--retries", "0", "--test-threads", "1", "--success-output", "immediate", "--failure-output", "immediate", "-E", selector], case, output, label, env, 180 if label == "original" else 300, check=False)
            output_text = (output / (label + ".stdout")).read_text(encoding="utf-8", errors="replace") + (output / (label + ".stderr")).read_text(encoding="utf-8", errors="replace")
            result["nonFirstPassMarkers"] = re.findall(r"(?m)^\s*(?:TRY [0-9]+|FLAKY|(?:FAIL|SLOW)\s+\+\s+LEAK|(?:\S*LEAK\S*)|TIMEOUT|ABORT)\b.*$", output_text)
            observation = {"label": label, "selected": selected, **result}
            results.append(observation)
            junit = target / "nextest/queue/junit.xml"
            if junit.exists():
                destination = output / (label + ".junit.xml")
                shutil.copy2(junit, destination)
                cases = [node for node in ET.parse(destination).iter("testcase") if node.find("skipped") is None]
                require(sorted(node.attrib["name"] for node in cases) == sorted(names), "JUnit does not prove the selected executions")
                observation["outcomes"] = execution_outcomes(cases)
            else:
                require(False, "missing fresh nextest JUnit")
            require(sha(binary) == binary_hash and sha(retained) == binary_hash, "artifact changed during tests")
            require(result["state"] != "timed_out", "runner timeout; do not start more observations with possible leftovers")
            require(not result["nonFirstPassMarkers"], "nextest reported retry, timeout, leak or abort; stop observations")
            require(all(item["permitsNextObservation"] for item in observation["outcomes"]),
                    "non-ordinary or unknown JUnit cause; stop observations with possible leftovers")
            has_failure = any(item["causes"] for item in observation["outcomes"])
            require(result["exit"] == (100 if has_failure else 0), "nextest exit does not match retained JUnit outcomes")
            junit.unlink()  # task-owned result only; keep the labeled retained copy
        receipt["status"] = "DIAGNOSTIC_ATTEMPT_COMPLETE" if all(item["exit"] == 0 and not item["nonFirstPassMarkers"] for item in results) else "DIAGNOSTIC_ATTEMPT_FAILED"
    finally:
        receipt["runs"] = results
        verify_source()
        receipt["sourceAndBinaryUnchanged"] = sha(binary) == binary_hash and sha(retained) == binary_hash
        require(receipt["sourceAndBinaryUnchanged"], "final artifact pin differs")
    return 0 if receipt["status"] == "DIAGNOSTIC_ATTEMPT_COMPLETE" else 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--case", type=Path, required=True)
    parser.add_argument("--case-commit", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--results", type=Path, required=True)
    args = parser.parse_args()
    args.results.mkdir(parents=True, exist_ok=False)
    receipt = {"status": "NOT_EXECUTED", "startedUnixNs": time.time_ns(), "issueFixed": False}
    try:
        return execute(args, receipt)
    except Exception as error:
        receipt.update(status="INFRASTRUCTURE_OR_PROVENANCE_FAILURE", error=f"{type(error).__name__}: {error}")
        return 2
    finally:
        receipt["finishedUnixNs"] = time.time_ns()
        write(args.results / "receipt.json", receipt)


if __name__ == "__main__":
    sys.exit(main())
