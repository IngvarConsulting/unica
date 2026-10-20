#!/usr/bin/env python3
"""One owned Windows first-attempt set; no rerun and no runtime policy changes."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sys
import time
import uuid
import xml.etree.ElementTree as ET

CASE = "cf2875a393e31158fa762c5f71000d9c39953c99"
RUNNER_SHA = "f995b2608359753120b1658dd0b831819ed35a3eb15f99717eda19fdcfa01fe4"
OWNER_PATH = "scripts/ci/probe-unica-wire.py"
OWNER_SHA = "b126d1905a80ec5f062d79206a9f1e3ddd8ce8ce94715d8f02b45e9c789810ad"
TESTS_PATH = "crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs"
SOURCES = {TESTS_PATH: "source/tests.rs", TESTS_PATH[:-3] + "/fixture_deadline_trace.rs": "source/fixture_deadline_trace.rs", TESTS_PATH[:-3] + "/staged_deadline_control.rs": "source/staged_deadline_control.rs"}
CAUSAL = "staged_publication_preserves_winner_after_original_read_deadline_expires"
EPOCHS = [[1000, 2000], [1100, 2100], [1200, 2200], [3900000, 3950000]]
PURPOSES = {"queued_staged_completed_transfer_preserves_exact_winner_and_reopens": "queued_completed", "queued_staged_failed_transfer_preserves_exact_winner_and_reopens": "queued_failed", "begun_staged_completed_transfer_preserves_exact_winner_and_reopens": "begun_completed", "queued_staged_cancelled_transfer_preserves_exact_winner_and_reopens": "queued_cancelled"}


def require(value, message):
    if not value:
        raise ValueError(message)


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def write(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=True, indent=2) + "\n", encoding="utf-8")


def error(error):
    return {"type": type(error).__name__, "message": str(error)}


def interrupt(error):
    seen = set()
    while error is not None and id(error) not in seen:
        if isinstance(error, KeyboardInterrupt):
            return True
        seen.add(id(error))
        error = error.__cause__
    return False


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def replace_bytes(path, data, results, mode=0o644):
    pending = results / ("source-pending-" + str(uuid.uuid4()))
    with pending.open("xb") as stream:
        stream.write(data)
        stream.flush()
    os.chmod(pending, mode)
    os.replace(pending, path)


def capture_case(name, native, stdout, stderr):
    result = {"name": name, "attempt": 1, "nativeExit": native.get("exit"), "timedOut": bool(native.get("timedOut")), "lifetimeConfirmed": native.get("lifetimeConfirmed"), "diagnosticComplete": False}
    result["nativeOutcome"] = "TIMEOUT" if result["timedOut"] else {0: "PASS", 101: "FAIL"}.get(result["nativeExit"], "PROCESS_ERROR")
    try:
        require(not result["timedOut"] and result["nativeExit"] in [0, 101], "timeout or non-libtest exit; preserve raw first outcome")
        require(stdout.count("running 1 test") == 1 and stdout.count("test " + name + " ... ") == 1, "exact libtest case identity/count missing")
        summaries = re.findall(r"(?m)^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;.*$", stdout)
        require(len(summaries) == 1, "libtest summary absent/duplicated")
        expected = ("ok", "1", "0", "0") if native["exit"] == 0 else ("FAILED", "0", "1", "0")
        require(summaries[0] == expected, "zero/ignored selection or libtest exit/summary disagreement")
        prefix = "STAGED_DEADLINE_CONTROL " if name.endswith("::" + CAUSAL) else "FIXTURE_DEADLINE "
        records = [json.loads(line[len(prefix):]) for line in stderr.splitlines() if line.startswith(prefix)]
        result["diagnosticRecords"] = records
        short = name.rsplit("::", 1)[1]
        if short == CAUSAL:
            require(len(records) == 1, "one mandatory causal record missing/duplicated")
            record = records[0]
            require(all(key in record for key in ["taskId", "invocationId", "receiptKeyDigest", "publication", "oldRead", "freshRead", "gateReceiveError"]), "causal record incomplete")
            if native["exit"] == 0:
                require(record["publication"] == {"Ok": "ok"} and record["oldRead"] == {"Err": "DeadlineExceeded"} and "Ok" in record["freshRead"], "PASS lacks actual causal result triple")
                require(record.get("gateReceiveError") is None and record.get("gateTimedOut") is False and record["deadlineNs"] == 5_000_000_000, "gate disposition/deadline differs")
                require(0 <= record["gateEnteredNs"] <= record["gateReleasedNs"] and record["gateReleasedNs"] >= record["deadlineNs"] and record["publicationReturnedNs"] >= record["deadlineNs"], "causal gate did not precede expired endpoint/returned publication")
            result["causalMechanismConfirmed"] = native["exit"] == 0
            result["historicalPublicationResult"] = "Unknown"
        else:
            retirement = short == "retirement_finalize_failure_stays_with_its_record"
            require((1 <= len(records) <= 4) if retirement else len(records) == 1, "mandatory fixture records missing/duplicated")
            if retirement:
                require([value.get("seedEpochs") for value in records] == EPOCHS[:len(records)] and all(value.get("purpose") == "retirement_seed" for value in records), "retirement seed prefix differs")
                if native["exit"] == 0:
                    require(len(records) == 4, "PASS omitted a seed")
            else:
                require(records[0].get("purpose") == PURPOSES[short], "staged case purpose differs")
            for record in records:
                require(all(record.get(key) is not None for key in ["taskId", "invocationId", "receiptKeyDigest", "deadlineNs", "phases"]), "fixture identity/phase envelope incomplete")
                require(record["deadlineNs"] == 5_000_000_000 and record["phases"], "fixture original endpoint/phases missing")
                previous = 0
                for phase in record["phases"]:
                    entered = phase.get("enteredNs")
                    returned = phase.get("returnedNs")
                    require(isinstance(entered, int) and entered >= previous, "phase monotonic ordering differs")
                    require(returned is None or isinstance(returned, int) and returned >= entered, "phase return precedes entry")
                    previous = entered if returned is None else returned
                if native["exit"] == 0:
                    require(record.get("unwinding") is False and all(phase.get("returnedNs") is not None and phase.get("failure") is None for phase in record["phases"]), "PASS has incomplete/failing fixture stage")
            result["observationScope"] = "retirement seed external transitions only" if retirement else "original staged transfer and original first get"
        result["diagnosticComplete"] = True
    except Exception as failure:
        result["captureError"] = error(failure)
    return result


def junit(path, case, duration, raw):
    fail = case.get("nativeOutcome") == "FAIL"
    broken = not case.get("diagnosticComplete") and not fail or case.get("nativeOutcome") in ["TIMEOUT", "PROCESS_ERROR"]
    suite = ET.Element("testsuite", name="windows-runtime-fixture-deadline", tests="1", failures=str(int(fail)), errors=str(int(broken)), time=f"{duration:.6f}")
    item = ET.SubElement(suite, "testcase", classname="unica-coder.lib", name=case["name"], time=f"{duration:.6f}")
    properties = ET.SubElement(item, "properties")
    for key in ["attempt", "nativeExit", "nativeOutcome", "diagnosticComplete", "timedOut"]:
        ET.SubElement(properties, "property", name=key, value=str(case.get(key)))
    if fail:
        ET.SubElement(item, "failure", message="first libtest attempt exited101; original raw retained").text = json.dumps(case.get("captureError"), ensure_ascii=True)
    elif broken:
        ET.SubElement(item, "error", message="capture/setup/process outcome; not a semantic RED").text = json.dumps(case.get("captureError"), ensure_ascii=True)
    ET.SubElement(item, "system-out").text = json.dumps(raw, ensure_ascii=True)
    ET.ElementTree(suite).write(path, encoding="utf-8", xml_declaration=True)


def finalize(case, results, originals, applied, pins, record, checked):
    failures = []
    first = interrupted = None
    safe = not record.get("unconfirmedCommandLifetimes")
    record["sourceRestorationDeferred"] = not safe
    record["restoration"] = []
    for relative, before in originals.items():
        target = case / relative
        item = {"path": relative, "originalAbsent": before is None, "attempted": safe}
        try:
            current = target.read_bytes() if os.path.lexists(target) and not target.is_symlink() else None
            if not safe:
                item["disposition"] = "retained-unconfirmed-command-lifetime"
            elif current == applied[relative] and before is None:
                target.unlink()
                item["disposition"] = "removed-known-new-test-source"
            elif current == applied[relative] and before is not None:
                replace_bytes(target, before, results)
                item["disposition"] = "restored-known-source"
            elif before == current and (before is not None or not os.path.lexists(target)):
                item["disposition"] = "already-original"
            else:
                raise ValueError("unexpected source state; never overwrite/remove")
            item["restoredByteExact"] = (not os.path.lexists(target)) if before is None else target.read_bytes() == before
            require(safe and item["restoredByteExact"], "source restoration deferred/incomplete")
        except BaseException as failure:
            first = failure if first is None else first
            interrupted = failure if interrupt(failure) else interrupted
            item["error"] = error(failure)
            failures.append({"phase": "source-restore", **item["error"], "path": relative})
        record["restoration"].append(item)
    record["inputPinsAfter"] = []
    for pin in pins:
        item = dict(pin)
        try:
            item["actualSha256"] = digest(case / pin["path"])
            require(item["actualSha256"] == item["sha256"], "input pin changed")
        except BaseException as failure:
            first = failure if first is None else first
            interrupted = failure if interrupt(failure) else interrupted
            item["error"] = error(failure)
            failures.append({"phase": "source-after-pin", "path": pin["path"], **error(failure)})
        record["inputPinsAfter"].append(item)
    try:
        checked(["git", "rev-parse", "HEAD"], "git-head-after", 15)
        record["headAfter"] = (results / "git-head-after.stdout").read_text().strip()
        require(record["headAfter"] == CASE, "case HEAD changed")
        checked(["git", "status", "--porcelain", "--untracked-files=no"], "git-status-after", 15)
        tracked = (results / "git-status-after.stdout").read_text(encoding="utf-8")
        record["trackedStatusAfter"] = tracked
        require(not tracked, "tracked case state changed")
        require(all(not os.path.lexists(case / relative) for relative, before in originals.items() if before is None), "new test source remains")
    except BaseException as failure:
        first = failure if first is None else first
        interrupted = failure if interrupt(failure) else interrupted
        failures.append({"phase": "source-status-after", **error(failure)})
    record["finalizationErrors"] = failures
    return failures, first, interrupted


def execute(args, results, record):
    case = args.case.resolve()
    payload = Path(__file__).resolve().parent
    require(args.case_commit == CASE and platform.system() == "Windows" and platform.machine().lower() in ["amd64", "x86_64"], "unreviewed case/platform")
    require(digest(payload / "manifest.json") == args.payload_manifest_sha256, "payload manifest differs")
    manifest = json.loads((payload / "manifest.json").read_text(encoding="utf-8"))
    names = json.loads((payload / "selection.json").read_text())["names"]
    require(len(names) == 6 and len(set(names)) == 6, "reviewed exact6 selection differs")
    record["plannedNames"] = names
    record["startedCases"] = []
    record["cases"] = []
    for member in manifest["members"]:
        path = (payload / member["path"]).resolve()
        require(path.is_relative_to(payload) and path.is_file() and digest(path) == member["sha256"], "payload member differs")
    require(digest(payload / "owned_commands.py") == RUNNER_SHA and digest(case / OWNER_PATH) == OWNER_SHA, "owned command source differs")
    contract = manifest["environmentContractSource"]
    require(digest(case / contract["path"]) == contract["sha256"], "environment input contract differs")
    runner = load("deadline_owned_commands", payload / "owned_commands.py")
    owner = load("deadline_process_ownership", case / OWNER_PATH)
    environment = dict(os.environ)
    for key in manifest["clearEnvironment"]:
        environment.pop(key, None)
    for name in ["temp", "xdg-config", "xdg-cache", "appdata", "localappdata"]:
        (results / name).mkdir()
    environment.update(CARGO_TARGET_DIR=str(results / "target"), CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="1", RUSTUP_TOOLCHAIN="stable", TMP=str(results / "temp"), TEMP=str(results / "temp"), XDG_CONFIG_HOME=str(results / "xdg-config"), XDG_CACHE_HOME=str(results / "xdg-cache"), APPDATA=str(results / "appdata"), LOCALAPPDATA=str(results / "localappdata"))
    record.update(caseCommit=CASE, casePath=str(case), resultsPath=str(results), explicitEnvironment={key: environment[key] for key in ["CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS", "RUSTUP_TOOLCHAIN", "TMP", "TEMP", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "APPDATA", "LOCALAPPDATA"]}, clearedEnvironment=manifest["clearEnvironment"], runnerSourceSha256=RUNNER_SHA, processOwnershipHelperSha256=OWNER_SHA, python=sys.version, machine=platform.platform(), provenance={key: os.environ.get(key) for key in ["GITHUB_SHA", "GITHUB_REF", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion"]})
    def checked(argv, label, timeout, cwd=case):
        return runner.checked_command(argv, cwd, results / label, timeout, environment, record, owner)
    checked(["git", "rev-parse", "HEAD"], "git-head-before", 15)
    require((results / "git-head-before.stdout").read_text().strip() == CASE, "case HEAD differs")
    checked(["git", "status", "--porcelain", "--untracked-files=no"], "git-status-before", 15)
    require(not (results / "git-status-before.stdout").read_text(), "tracked case already dirty")
    pins = json.loads((payload / "input-pins.json").read_text())["pins"]
    require(len(pins) == 43, "selected source43 differs")
    record["inputPinsBefore"] = []
    for pin in pins:
        item = {**pin, "actualSha256": digest(case / pin["path"])}
        record["inputPinsBefore"].append(item)
        require(item["actualSha256"] == item["sha256"], "source43 pin differs")
    for label, argv in [("rustc", ["rustc", "+stable", "--version", "--verbose"]), ("cargo", ["cargo", "+stable", "--version", "--verbose"]), ("nextest", ["cargo", "+stable", "nextest", "--version"])]:
        checked(argv, "version-" + label, 30)
    require("cargo-nextest 0.9.148" in (results / "version-nextest.stdout").read_text(), "nextest version differs")
    originals, applied = {}, {}
    for relative, filename in SOURCES.items():
        target = case / relative
        require(target.resolve().is_relative_to(case) and not target.is_symlink(), "source target escapes owned case")
        require(target.is_file() if relative == TESTS_PATH else not os.path.lexists(target), "unexpected original test source state")
        originals[relative] = target.read_bytes() if relative == TESTS_PATH else None
        applied[relative] = (payload / filename).read_bytes()
        (results / (Path(relative).name + ".applied.rs")).write_bytes(applied[relative])
        if originals[relative] is not None:
            (results / (Path(relative).name + ".original.rs")).write_bytes(originals[relative])
    record["sourceImages"] = [{"path": path, "originalAbsent": before is None, "originalSha256": hashlib.sha256(before).hexdigest() if before is not None else None, "appliedSha256": hashlib.sha256(applied[path]).hexdigest()} for path, before in originals.items()]
    primary = None
    try:
        for relative, data in applied.items():
            replace_bytes(case / relative, data, results)
        checked(["cargo", "+stable", "test", "--locked", "-p", "unica-coder", "--lib", "--no-run", "--message-format=json"], "build", 2400)
        messages = []
        for line in (results / "build.stdout").read_text(encoding="utf-8").splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if item.get("reason") == "compiler-artifact" and item.get("profile", {}).get("test") and item.get("executable") and Path(item["target"]["src_path"]).resolve() == case / "crates/unica-coder/src/lib.rs":
                messages.append(item)
        require(len(messages) == 1 and messages[0].get("fresh") is False and Path(messages[0].get("manifest_path", "")).resolve() == case / "crates/unica-coder/Cargo.toml", "no fresh case-owned libtest compiler artifact")
        compiled = Path(messages[0]["executable"]).resolve()
        require(compiled.is_relative_to(results / "target"), "compiled binary outside owned target")
        retained = results / "owned-libtest.exe"
        shutil.copy2(compiled, retained)
        binary_sha = digest(retained)
        require(binary_sha == digest(compiled), "retained binary differs")
        record["artifact"] = {"compilerMessage": messages[0], "retainedPath": str(retained), "retainedSha256": binary_sha, "bytes": retained.stat().st_size}
        expression = " | ".join("test(/^" + re.escape(name) + "$/)" for name in names)
        checked(["cargo", "+stable", "nextest", "list", "--locked", "-p", "unica-coder", "--lib", "--profile", "queue", "--message-format", "json", "-E", expression], "list", 600)
        listed = json.loads((results / "list.stdout").read_text(encoding="utf-8"))
        selected = []
        for suite in listed["rust-suites"].values():
            for name, test in suite["testcases"].items():
                if test.get("filter-match", {}).get("status") == "matches":
                    require(not test.get("ignored") and Path(suite["binary-path"]).resolve() == compiled, "selected suite binary/ignored state differs")
                    require(suite.get("package-name") == "unica-coder" and suite.get("kind") == "lib", "selected package/kind differs")
                    selected.append({"name": name, "binary": suite["binary-id"], "cwd": str(Path(suite["cwd"]).resolve())})
        require(len(selected) == 6 and {item["name"] for item in selected} == set(names), "actual queue list is not exact6")
        require(digest(compiled) == binary_sha and digest(retained) == binary_sha, "nextest list changed own binary")
        record["selectionActual"] = selected
        write(results / "actual-plan.json", selected)
        for number, name in enumerate(names, 1):
            require(all(digest(case / path) == hashlib.sha256(data).hexdigest() for path, data in applied.items()), "applied source changed before first test")
            require(digest(retained) == binary_sha, "retained executable changed")
            cwd = Path(next(item["cwd"] for item in selected if item["name"] == name))
            require(cwd == case / "crates/unica-coder", "actual suite cwd differs")
            prefix = results / f"test-{number:02d}"
            started = time.monotonic()
            record["startedCases"].append({"name": name, "attempt": 1, "prefix": str(prefix)})
            try:
                native = runner.command([retained, "--exact", name, "--nocapture", "--test-threads=1"], cwd, prefix, 600, environment, record, owner)
            except BaseException as failure:
                native = record["commands"][-1]
                captured = {"name": name, "attempt": 1, "nativeExit": native.get("exit"), "timedOut": bool(native.get("timedOut")), "nativeOutcome": "TIMEOUT" if native.get("timedOut") else {0: "PASS", 101: "FAIL"}.get(native.get("exit"), "PROCESS_ERROR"), "diagnosticComplete": False, "captureError": error(failure), "commandReceipt": str(prefix.with_suffix(".json")), "durationSeconds": time.monotonic() - started}
                record["cases"].append(captured)
                if captured["nativeOutcome"] != "PASS":
                    record.setdefault("firstNativeFailure", {"name": name, "nativeExit": captured["nativeExit"], "nativeOutcome": captured["nativeOutcome"]})
                try:
                    write(results / f"case-{number:02d}.json", captured)
                    junit(results / f"case-{number:02d}.junit.xml", captured, captured["durationSeconds"], {"commandReceipt": captured["commandReceipt"]})
                except BaseException as secondary:
                    record.setdefault("secondaryStorageErrors", []).append(error(secondary))
                    if interrupt(secondary) and not interrupt(failure):
                        raise secondary from failure
                raise
            captured = capture_case(name, native, prefix.with_suffix(".stdout").read_text(encoding="utf-8", errors="replace"), prefix.with_suffix(".stderr").read_text(encoding="utf-8", errors="replace"))
            captured.update(durationSeconds=time.monotonic() - started, commandReceipt=str(prefix.with_suffix(".json")), binarySha256=binary_sha)
            record["cases"].append(captured)
            if captured["nativeOutcome"] != "PASS":
                record.setdefault("firstNativeFailure", {"name": name, "nativeExit": captured["nativeExit"], "nativeOutcome": captured["nativeOutcome"]})
            if not captured["diagnosticComplete"]:
                record.setdefault("firstCaptureFailure", {"name": name, "error": captured.get("captureError")})
            write(results / f"case-{number:02d}.json", captured)
            junit(results / f"case-{number:02d}.junit.xml", captured, captured["durationSeconds"], {"stdout": str(prefix.with_suffix(".stdout")), "stdoutSha256": native.get("stdoutSha256"), "stderr": str(prefix.with_suffix(".stderr")), "stderrSha256": native.get("stderrSha256")})
        record["notRun"] = [name for name in names if name not in {item["name"] for item in record["cases"]}]
        require(not record["notRun"], "case set incomplete")
        return 2 if any(not item["diagnosticComplete"] for item in record["cases"]) else 1 if any(item["nativeOutcome"] != "PASS" for item in record["cases"]) else 0
    except BaseException as failure:
        primary = failure
        record.setdefault("firstError", error(failure))
        raise
    finally:
        failures, first, interrupted = finalize(case, results, originals, applied, pins, record, checked)
        if failures:
            record["captureIncomplete"] = True
            if interrupted is not None and (primary is None or not interrupt(primary)):
                raise interrupted from primary
            if primary is None:
                raise first


def main(args):
    results = args.results.resolve()
    require(not os.path.lexists(results), "results path already exists; never overwrite evidence")
    results.mkdir(parents=True, exist_ok=False)
    record = {"status": "CAPTURE_INCOMPLETE", "plannedCases": 6, "attemptsPerCase": 1, "retries": 0, "failFast": False, "caseResults": "one first execution each; setup/capture failures remain distinct"}
    code = 2
    try:
        code = execute(args, results, record)
        record["status"] = {0: "PASS", 1: "OBSERVED_FAIL", 2: "CAPTURE_INCOMPLETE"}[code]
        if code == 2:
            record["captureIncomplete"] = True
    except BaseException as failure:
        code = 130 if interrupt(failure) else 2
        record.setdefault("firstError", error(failure))
        record["captureIncomplete"] = True
        record["error"] = error(failure)
        record["caseAndDataRetained"] = bool(record.get("unconfirmedCommandLifetimes") or record.get("finalizationErrors"))
    finally:
        started = {item["name"] for item in record.get("startedCases", [])}
        finished = {item["name"] for item in record.get("cases", [])}
        record["notRun"] = [name for name in record.get("plannedNames", []) if name not in started]
        record["incompleteStartedCases"] = sorted(started - finished)
        record["actualCasesStarted"] = len(started)
        record["plannedCasesNotStartedCount"] = 6 - len(started)
        record["exit"] = code
        try:
            write(results / "receipt.json", record)
        except BaseException as failure:
            if code != 130:
                code = 130 if interrupt(failure) else 2
            record["captureIncomplete"] = True
            record["status"] = "CAPTURE_INCOMPLETE"
            record["exit"] = code
            try:
                print("receipt capture failed: " + str(failure), file=sys.stderr)
            except BaseException as secondary:
                if interrupt(secondary):
                    code = 130
    return code


def safe_main(args):
    try:
        return main(args)
    except BaseException as failure:
        code = 130 if interrupt(failure) else 2
        try:
            print("diagnostic setup/capture failed: " + str(failure), file=sys.stderr)
        except BaseException as secondary:
            if interrupt(secondary):
                code = 130
        return code


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--case", type=Path, required=True)
    parser.add_argument("--case-commit", required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--payload-manifest-sha256", required=True)
    sys.exit(safe_main(parser.parse_args()))
