#!/usr/bin/env python3
"""Pinned, single-attempt Windows #1523 diagnostics; never changes case sources."""
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

FEATURE = "receipt-ledger-test-support"
LIB = "unica_coder"
INTEGRATION = "daemon_receipt_ledger"
CONTROLS = [
    "infrastructure::receipt_ledger::tests::cancelled_batch_actor_ticket_waits_for_real_publication_before_reply",
    "infrastructure::daemon::runtime_v5::receipt_scenario_v5::tests::late_exact_submit_keeps_original_lazy_cancel_deadline_and_all_durable_winners",
    "infrastructure::daemon::runtime_v5::receipt_scenario_v5::tests::fresh_post_restart_projection_detects_missing_old_non_exact_task",
    "infrastructure::task_lifecycle_link_store_v5::tests::causal_observer_keeps_first_typed_io_cause_when_event_buffer_overflows",
]
LAZY = "thirty_two_lazy_cancel_actor_batch_finishes_within_125ms"
TASK = "task_store_4097_materialization_preserves_existing_tasks_listener_and_recovery"
BRANCHES = ["TaskPromisedActorBound", "TaskHandoffActorBoundBegun"]
PLAN = [("controls", LIB, CONTROLS, "queue", 2460),
        ("lazy32", INTEGRATION, [LAZY], "large", 1860),
        ("task4097", INTEGRATION, [TASK], "large", 1860)]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, value):
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")


def exact_selection(listed, binaries, target, names):
    selected = []
    for suite in listed["rust-suites"].values():
        path = Path(suite["binary-path"]).resolve()
        require(path in binaries.values(), "selection contains an unknown binary")
        for name, entry in suite["testcases"].items():
            if entry["filter-match"]["status"] == "matches":
                require(not entry["ignored"], "selected test is ignored")
                selected.append((path, name))
    require(sorted(selected) == sorted((binaries[target], name) for name in names),
            "missing, duplicate, wrong-binary or unexpected selected test")
    return [name for _, name in selected]


def junit_observation(path, names):
    nodes = list(ET.parse(path).iter("testcase"))
    cases = []
    for node in nodes:
        causes = [{"kind": child.tag, "type": child.get("type"),
                   "message": child.get("message"), "text": "".join(child.itertext())}
                  for child in node if child.tag in {"failure", "error"}]
        cases.append({"name": node.attrib["name"], "skipped": node.find("skipped") is not None,
                      "failed": bool(causes), "causes": causes,
                      "retried": any(node.find(tag) is not None for tag in
                                     ("rerunFailure", "rerunError", "flakyFailure", "flakyError"))})
    executed = [node["name"] for node in cases if not node["skipped"]]
    return {"cases": cases, "executed": executed,
            "failedTests": [node["name"] for node in cases if node["failed"]],
            "missingSelected": sorted(set(names)-set(executed))}


def validate_junit(observed, names, exit_code, known_ignored=()):
    nodes = observed["cases"]
    require(len({node["name"] for node in nodes}) == len(nodes), "duplicate JUnit case")
    require(sorted(observed["executed"]) == sorted(names),
            "JUnit does not prove every exact selected test once")
    # The pinned repository has report-skipped="ignored". Those inventory-known
    # nonselected entries are retained, never counted as executions or controls.
    require(all(not node["skipped"] or (node["name"] in known_ignored and node["name"] not in names)
                for node in nodes), "selected or unknown JUnit case was skipped")
    require(all(not node["retried"] for node in nodes), "JUnit contains a retry")
    failures = observed["failedTests"]
    require((exit_code == 0) == (not failures), "exit status disagrees with JUnit")
    return failures


def junit_cases(path, names, exit_code, known_ignored=()):
    return validate_junit(junit_observation(path, names), names, exit_code, known_ignored)


def continuation_decision(observed, exit_code, text, label):
    # Pinned nextest 0.9.148 (cd1d6d5467322dbc0d39a80cb671d000ac302798):
    # nextest-metadata/src/exit_codes.rs and reporter/aggregator/junit.rs.
    # Rust libtest failure 101 becomes nextest TEST_RUN_FAILED=100. Other
    # failure/error types do not establish an ordinary completed test failure.
    # Match reporter status + bracketed duration, not arbitrary assertion prose.
    status_lines = list(re.finditer(
        r"(?m)^[ \t]*(?P<status>[A-Z][A-Z0-9 /+\-]*?)[ \t]+"
        r"\[[ \t]*>?[ \t]*[0-9]+\.[0-9]+s\][ \t]+\S[^\n]*$", text))
    unsafe = [match.group(0) for match in status_lines
              if match.group("status") not in {"PASS", "FAIL", "SLOW"}]
    failed = [node for node in observed["cases"] if node["failed"]]
    if unsafe or any(node["retried"] for node in observed["cases"]):
        outcome = "UNSAFE_OR_RETRIED"
    elif exit_code == 0 and not failed:
        outcome = "PASS"
    elif exit_code == 100 and failed and all(
            len(node["causes"]) == 1 and node["causes"][0]["kind"] == "failure"
            and node["causes"][0]["type"] == "test failure with exit code 101"
            for node in failed):
        outcome = "ORDINARY_TEST_FAILURE"
    else:
        outcome = "UNCLASSIFIED_FAILURE"
    return {"outcome": outcome, "unsafeStatusLines": unsafe,
            "mayContinue": outcome == "PASS" or
            (outcome == "ORDINARY_TEST_FAILURE" and label in {"lazy32", "task4097"})}


def observations(text, label, passed):
    branches = re.findall(r"(?m)^\s*TASK4097_BRANCH (\S+)\s*$", text)
    traces = [json.loads(value) for value in re.findall(r"(?m)^\s*RECEIPT_CAUSAL (\{[^\n]*\})\s*$", text)]
    for item in traces:
        require(isinstance(item["label"], str) and type(item["succeeded"]) is bool, "invalid trace header")
        trace = item["trace"]
        require(trace["capacity"] == 256 and type(trace["overflow"]) is int and trace["overflow"] >= 0,
                "unexpected trace buffer contract")
        require(type(trace["complete"]) is bool and trace["complete"] == (trace["overflow"] == 0)
                and isinstance(trace["events"], list) and len(trace["events"]) <= 256,
                "invalid trace completeness")
        if passed:
            require(trace["events"], "successful observed operation has an empty trace")
    if label == "lazy32":
        require(not branches and len(traces) <= 1
                and all(item["label"] == "lazy_cancel_storm" for item in traces), "unexpected lazy trace")
        if passed:
            require(len(traces) == 1 and traces[0]["succeeded"], "successful lazy test lacks exact trace")
    elif label == "task4097":
        require(branches == BRANCHES[:len(branches)] and len(branches) <= 2,
                "Task4097 branch markers duplicated, reordered or unknown")
        require(len(traces) <= len(branches) and all(item["label"] == "materialized" for item in traces),
                "unexpected Task4097 trace")
        if passed:
            require(branches == BRANCHES and len(traces) == 2 and all(item["succeeded"] for item in traces),
                    "successful Task4097 lacks both branch observations")
    return {"branches": branches, "traces": traces,
            "noBufferOverflow": all(item["trace"]["complete"] for item in traces),
            "bothTaskBranchesObserved": label == "task4097" and branches == BRANCHES and len(traces) == 2,
            "limits": "complete means no buffer overflow, not exhaustive OS causes or worker completion"}


def stop_tree(process, output, label, deadline):
    # No Windows Job ownership claim: taskkill targets the still-known process
    # tree; a disposable Actions job contains unprovable residual descendants.
    result = {"allDescendantsConfirmedDead": False}
    with (output / f"{label}.cleanup.stdout").open("xb") as out, (output / f"{label}.cleanup.stderr").open("xb") as err:
        try:
            completed = subprocess.run(["taskkill.exe", "/PID", str(process.pid), "/T", "/F"],
                                       stdout=out, stderr=err, timeout=max(0.01, min(20, deadline-time.monotonic())))
            result["taskkillExit"] = completed.returncode
        except (OSError, subprocess.TimeoutExpired) as error:
            result["taskkillError"] = f"{type(error).__name__}: {error}"
        try:
            if process.poll() is None:
                process.kill()
            result["directChildExit"] = process.wait(timeout=max(0.01, deadline-time.monotonic()))
        except (OSError, subprocess.TimeoutExpired) as error:
            result["waitError"] = f"{type(error).__name__}: {error}"
    return result


def run(argv, cwd, output, label, env, timeout, check=True):
    started = time.monotonic()
    item = {"argv": list(map(str, argv)), "cwd": str(cwd), "startedUnixNs": time.time_ns(),
            "outerLimitSeconds": timeout, "cleanupReserveSeconds": 30, "state": "started"}
    write(output / f"{label}.started.json", item)
    deadline = started + timeout
    process = None
    try:
        with (output / f"{label}.stdout").open("xb") as stdout, (output / f"{label}.stderr").open("xb") as stderr:
            process = subprocess.Popen(item["argv"], cwd=cwd, env=env, stdout=stdout, stderr=stderr,
                                       creationflags=subprocess.CREATE_NEW_PROCESS_GROUP)
            item["pid"] = process.pid
            write(output / f"{label}.launch.json", item)
            try:
                remaining = deadline - 30 - time.monotonic()
                if remaining <= 0:
                    raise subprocess.TimeoutExpired(item["argv"], timeout - 30)
                code = process.wait(timeout=remaining)
                if time.monotonic() > deadline - 30:
                    raise subprocess.TimeoutExpired(item["argv"], timeout - 30)
                item.update(state="completed", exit=code)
            except subprocess.TimeoutExpired:
                item.update(state="timed_out", exit=None,
                            cleanup=stop_tree(process, output, label, deadline))
    except Exception as error:
        item.update(state="runner_error", exit=None, error=f"{type(error).__name__}: {error}")
        if process is not None and process.poll() is None:
            item["cleanup"] = stop_tree(process, output, label, deadline)
    finally:
        item.update(finishedUnixNs=time.time_ns(), commandElapsedSeconds=time.monotonic()-started)
        for stream in ["stdout", "stderr", "cleanup.stdout", "cleanup.stderr"]:
            path = output / f"{label}.{stream}"
            if path.exists():
                item[stream + "Sha256"] = sha(path)
        write(output / f"{label}.json", item)
    if check:
        require(item.get("exit") == 0 and item["state"] == "completed", f"{label}: {item}")
    return item


def execute(args, receipt):
    case, output = args.case.resolve(), args.results.resolve()
    payload = Path(__file__).resolve().parent
    control = payload.parents[2]
    manifest_path = payload / "manifest.json"
    require(sha(manifest_path) == args.manifest_sha256, "control manifest differs")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    require(re.fullmatch(r"[0-9a-f]{40}", manifest["caseCommit"]), "root must freeze the reviewed case commit")
    require(args.case_commit == manifest["caseCommit"], "unreviewed case commit")
    require(sha(__file__) == manifest["runnerSha256"], "runner differs")
    require(sha(control / ".github/workflows/unica-plugin-release.yml") == manifest["workflowSha256"], "workflow differs")
    require(platform.system() == "Windows" and platform.machine().lower() in {"amd64", "x86_64"}, "native Windows x64 required")

    def git(root, *argv):
        return subprocess.check_output(["git", *argv], cwd=root, text=True).strip()

    require(git(case, "rev-parse", "HEAD") == args.case_commit, "case checkout differs")
    require(git(control, "rev-parse", "HEAD") == os.environ["GITHUB_SHA"], "control checkout differs")
    require(os.environ.get("GITHUB_RUN_ATTEMPT") == "1", "workflow rerun is not a first diagnostic attempt")

    def verify_source():
        for root in (case, control):
            require(not git(root, "status", "--porcelain", "--untracked-files=no"), "tracked checkout changed")
            require(git(root, "config", "--get", "core.autocrlf") == "false", "autocrlf must be false before checkout")
        for path, expected in manifest["sourcePins"].items():
            require(sha(case / path) == expected, "case pin differs: " + path)

    verify_source()
    receipt.update(caseCommit=args.case_commit, controlCommit=os.environ["GITHUB_SHA"], manifestSha256=sha(manifest_path),
                   os=platform.platform(), machine=platform.machine(), sourcePins=manifest["sourcePins"],
                   provenance={k: os.environ.get(k) for k in ["GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "GITHUB_REF", "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion"]})
    env = dict(os.environ)
    for name in list(env):
        if name.startswith("NEXTEST_") or name in {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTC_BOOTSTRAP", "CARGO_BUILD_TARGET"}:
            env.pop(name)
    target = output / "target"
    require(not target.exists(), "target must be fresh and task-owned")
    env.update(CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="0",
               CARGO_TERM_COLOR="never", UNICA_RECEIPT_CAUSAL_TRACE="1")
    receipt["explicitEnvironment"] = {k: env[k] for k in ["CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "CARGO_TERM_COLOR", "UNICA_RECEIPT_CAUSAL_TRACE"]}
    for label, argv in [("rustc-version", ["rustc", "-Vv"]), ("cargo-version", ["cargo", "-V"]), ("nextest-version", ["cargo", "nextest", "--version"])]:
        run(argv, case, output, label, env, 60)
    require(re.search(r"\bcargo-nextest 0\.9\.148\b", (output / "nextest-version.stdout").read_text()), "nextest version differs")
    targets = ["--locked", "-p", "unica-coder", "--features", FEATURE, "--lib", "--test", INTEGRATION]
    run(["cargo", "test", *targets, "--no-run", "--message-format=json"], case, output, "build", env, 2400)
    artifacts = {}
    sources = {LIB: case / "crates/unica-coder/src/lib.rs",
               INTEGRATION: case / "crates/unica-coder/tests/daemon_receipt_ledger.rs"}
    for line in (output / "build.stdout").read_text(encoding="utf-8").splitlines():
        item = json.loads(line)
        if item.get("reason") != "compiler-artifact" or not item.get("profile", {}).get("test") or not item.get("executable"):
            continue
        for name, source in sources.items():
            if Path(item["target"]["src_path"]).resolve() == source:
                require(name not in artifacts and item["fresh"] is False, "one fresh artifact per target required")
                binary = Path(item["executable"]).resolve()
                require(binary.is_relative_to(target) and binary.suffix == ".exe", "binary outside task-owned target")
                retained = output / f"retained-{name}.exe"
                shutil.copy2(binary, retained)
                artifacts[name] = {"compilerMessage": item, "path": str(binary), "sha256": sha(binary), "retainedPath": str(retained)}
    require(set(artifacts) == set(sources), "two exact feature-enabled binaries required")
    receipt["artifacts"] = artifacts
    write(output / "artifacts.json", artifacts)
    binaries = {name: Path(item["path"]) for name, item in artifacts.items()}

    def verify_binaries():
        for item in artifacts.values():
            require(sha(item["path"]) == sha(item["retainedPath"]) == item["sha256"], "binary changed")

    # Preparation only: official binaries metadata may call Cargo once more.
    # All later list/run commands reuse this metadata and must preserve hashes.
    run(["cargo", "metadata", "--locked", "--features", "unica-coder/" + FEATURE, "--format-version", "1"], case, output, "cargo-metadata", env, 150)
    run(["cargo", "nextest", "list", *targets, "--list-type", "binaries-only", "--message-format", "json"], case, output, "binaries-metadata", env, 630)
    metadata = json.loads((output / "binaries-metadata.stdout").read_text())["rust-binaries"]
    require(len(metadata) == 2 and {Path(item["binary-path"]).resolve() for item in metadata.values()} == set(binaries.values()), "nextest binaries differ")
    verify_binaries()
    reuse = ["--cargo-metadata", str(output / "cargo-metadata.stdout"), "--binaries-metadata", str(output / "binaries-metadata.stdout"), "--user-config-file", "none"]
    receipt["runs"] = []
    try:
        # Verify all six names before executing any test.
        ignored_inventory = {}
        for label, binary_name, names, profile, _ in PLAN:
            selector = "binary(=" + binary_name + ") & (" + " | ".join("test(=" + name + ")" for name in names) + ")"
            run(["cargo", "nextest", "list", *reuse, "--profile", profile, "-E", selector, "--message-format", "json"], case, output, label + "-selection", env, 90)
            listed = json.loads((output / (label + "-selection.stdout")).read_text())
            exact_selection(listed, binaries, binary_name, names)
            ignored_inventory[label] = {name for suite in listed["rust-suites"].values()
                                        for name, entry in suite["testcases"].items() if entry["ignored"]}
        for label, binary_name, names, profile, limit in PLAN:
            verify_source(); verify_binaries()
            selector = "binary(=" + binary_name + ") & (" + " | ".join("test(=" + name + ")" for name in names) + ")"
            junit = target / "nextest" / profile / "junit.xml"
            require(not junit.exists(), "stale nextest report")
            result = run(["cargo", "nextest", "run", *reuse, "--profile", profile, "--retries", "0", "--test-threads", "1", "--success-output", "immediate", "--failure-output", "immediate", "-E", selector], case, output, label, env, limit, check=False)
            item = {"label": label, "names": names, "profile": profile, **result,
                    "testOutcome": "EXIT_ZERO" if result.get("exit") == 0 else "FAILED_OR_INTERRUPTED"}
            receipt["runs"].append(item)
            if junit.exists():
                retained_junit = output / (label + ".junit.xml")
                shutil.copy2(junit, retained_junit)
                item["junitSha256"] = sha(retained_junit)
            require(result["state"] == "completed", "outer failure: no further test may start")
            require(junit.exists(), "missing fresh JUnit")
            item["junitObserved"] = junit_observation(retained_junit, names)
            item["failedTests"] = item["junitObserved"]["failedTests"]
            validate_junit(item["junitObserved"], names, result["exit"], ignored_inventory[label])
            text = (output / (label + ".stdout")).read_text(encoding="utf-8", errors="replace") + "\n" + (output / (label + ".stderr")).read_text(encoding="utf-8", errors="replace")
            item["continuation"] = continuation_decision(item["junitObserved"], result["exit"], text, label)
            require(item["continuation"]["mayContinue"],
                    f"{label}: unsafe result or failed causal control gate; STOP: {item['continuation']}")
            item["observations"] = observations(text, label, result["exit"] == 0)
            verify_binaries()
            junit.unlink()  # Only the task-owned result; labeled retained copy stays.
            if label == "controls":
                require(result["exit"] == 0, "causal control gate failed; originals not authorized")
        all_passed = all(item["exit"] == 0 for item in receipt["runs"])
        no_overflow = all(item["observations"]["noBufferOverflow"] for item in receipt["runs"])
        receipt["status"] = ("DIAGNOSTIC_ATTEMPT_COMPLETE" if all_passed and no_overflow else "DIAGNOSTIC_ATTEMPT_FAILED_OR_INCOMPLETE")
        return 0 if receipt["status"] == "DIAGNOSTIC_ATTEMPT_COMPLETE" else 1
    finally:
        verify_source(); verify_binaries()
        receipt["sourceAndBinariesUnchanged"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", type=Path, required=True)
    parser.add_argument("--case-commit", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--results", type=Path, required=True)
    args = parser.parse_args()
    args.results.mkdir(parents=True, exist_ok=False)
    receipt = {"status": "NOT_EXECUTED", "startedUnixNs": time.time_ns(), "issueFixed": False,
               "limits": ["TRACE1 is diagnostic, not trace-disabled timing acceptance", "no retries or resume", "no full Windows large-suite acceptance", "timeout tree containment is not a successful owned-child cleanup proof"]}
    try:
        return execute(args, receipt)
    except Exception as error:
        receipt.update(status="STOPPED_WITH_RETAINED_EVIDENCE", error=f"{type(error).__name__}: {error}")
        return 2
    finally:
        receipt["finishedUnixNs"] = time.time_ns()
        write(args.results / "receipt.json", receipt)


if __name__ == "__main__":
    sys.exit(main())
