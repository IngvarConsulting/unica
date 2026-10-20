#!/usr/bin/env python3
"""One owned first Windows execution of the manual-cancel fixture candidate."""
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
import xml.etree.ElementTree as ET

CASE = "b471028e270826bc03ee163efe91a876d65bbf96"
RUNNER_SHA = "f995b2608359753120b1658dd0b831819ed35a3eb15f99717eda19fdcfa01fe4"
OWNER_PATH = "scripts/ci/probe-unica-wire.py"
OWNER_SHA = "b126d1905a80ec5f062d79206a9f1e3ddd8ce8ce94715d8f02b45e9c789810ad"


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


def capture_case(name, native, stdout):
    result = {"name": name, "attempt": 1, "nativeExit": native.get("exit"), "timedOut": bool(native.get("timedOut")), "lifetimeConfirmed": native.get("lifetimeConfirmed"), "diagnosticComplete": False}
    result["nativeOutcome"] = "TIMEOUT" if result["timedOut"] else {0: "PASS", 101: "FAIL"}.get(result["nativeExit"], "PROCESS_ERROR")
    try:
        require(not result["timedOut"] and result["nativeExit"] in [0, 101], "timeout or non-libtest exit; preserve raw first outcome")
        require(stdout.count("running 1 test") == 1 and stdout.count("test " + name + " ... ") == 1, "exact libtest case identity/count missing")
        summaries = re.findall(r"(?m)^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;.*$", stdout)
        require(len(summaries) == 1, "libtest summary absent/duplicated")
        expected = ("ok", "1", "0", "0") if native["exit"] == 0 else ("FAILED", "0", "1", "0")
        require(summaries[0] == expected, "zero/ignored selection or libtest exit/summary disagreement")
        result["diagnosticComplete"] = True
    except Exception as failure:
        result["captureError"] = error(failure)
    return result


def junit(path, case, duration, raw):
    fail = case.get("nativeOutcome") == "FAIL"
    broken = not case.get("diagnosticComplete") and not fail or case.get("nativeOutcome") in ["TIMEOUT", "PROCESS_ERROR"]
    suite = ET.Element("testsuite", name="windows-manual-cancel-init-flush", tests="1", failures=str(int(fail)), errors=str(int(broken)), time=f"{duration:.6f}")
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


def finalize(case, results, pins, record, checked):
    failures = []
    first = interrupted = None
    safe = not record.get("unconfirmedCommandLifetimes")
    record["sourceAfterStable"] = safe
    record["inputPinsAfter"] = []
    for pin in pins:
        item = dict(pin)
        try:
            item["actualSha256"] = digest(case / pin["path"])
            require(item["actualSha256"] == item["sha256"], "candidate source pin changed")
        except BaseException as failure:
            first = failure if first is None else first
            interrupted = failure if interrupt(failure) else interrupted
            item["error"] = error(failure)
            failures.append({"phase": "source-after-pin", "path": pin["path"], **error(failure)})
        record["inputPinsAfter"].append(item)
    try:
        require(safe, "unknown command lifetime; source after observation is not stable")
        checked(["git", "rev-parse", "HEAD"], "git-head-after", 15)
        record["headAfter"] = (results / "git-head-after.stdout").read_text().strip()
        require(record["headAfter"] == CASE, "candidate HEAD changed")
        checked(["git", "status", "--porcelain", "--untracked-files=no"], "git-status-after", 15)
        record["trackedStatusAfter"] = (results / "git-status-after.stdout").read_text(encoding="utf-8")
        require(not record["trackedStatusAfter"], "tracked candidate state changed")
    except BaseException as failure:
        first = failure if first is None else first
        interrupted = failure if interrupt(failure) else interrupted
        failures.append({"phase": "source-status-after", **error(failure)})
    record["finalizationErrors"] = failures
    record["sourceAfterStable"] = not failures and not record.get("unconfirmedCommandLifetimes")
    return failures, first, interrupted


def execute(args, results, record):
    case = args.case.resolve()
    payload = Path(__file__).resolve().parent
    require(re.fullmatch(r"[0-9a-f]{40}", CASE) and args.case_commit == CASE and platform.system() == "Windows" and platform.machine().lower() in ["amd64", "x86_64"], "unreviewed case/platform")
    require(digest(payload / "manifest.json") == args.payload_manifest_sha256, "payload manifest differs")
    manifest = json.loads((payload / "manifest.json").read_text(encoding="utf-8"))
    require(manifest.get("ready") is True, "draft payload is not executable")
    require(os.environ.get("GITHUB_RUN_ATTEMPT") == "1", "only the first Actions attempt is admissible")
    require(manifest["caseCommit"] == CASE and manifest["selectedCases"] == 1, "candidate payload identity differs")
    names = json.loads((payload / "selection.json").read_text())["names"]
    require(len(names) == 1 and len(set(names)) == 1, "reviewed exact1 selection differs")
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
    require(not os.path.lexists(results / "target"), "test target must be fresh")
    environment.update(CARGO_TARGET_DIR=str(results / "target"), CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="1", RUSTUP_TOOLCHAIN="stable", TMP=str(results / "temp"), TEMP=str(results / "temp"), XDG_CONFIG_HOME=str(results / "xdg-config"), XDG_CACHE_HOME=str(results / "xdg-cache"), APPDATA=str(results / "appdata"), LOCALAPPDATA=str(results / "localappdata"))
    record.update(caseCommit=CASE, casePath=str(case), resultsPath=str(results), explicitEnvironment={key: environment[key] for key in ["CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS", "RUSTUP_TOOLCHAIN", "TMP", "TEMP", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "APPDATA", "LOCALAPPDATA"]}, clearedEnvironment=manifest["clearEnvironment"], runnerSourceSha256=RUNNER_SHA, processOwnershipHelperSha256=OWNER_SHA, python=sys.version, machine=platform.platform(), provenance={key: os.environ.get(key) for key in ["GITHUB_SHA", "GITHUB_REF", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion"]})
    def checked(argv, label, timeout, cwd=case):
        return runner.checked_command(argv, cwd, results / label, timeout, environment, record, owner)
    inputs = json.loads((payload / "input-pins.json").read_text())
    require(inputs["caseCommit"] == CASE, "candidate input commit differs")
    pins = inputs["pins"]
    require(len(pins) == 14 and manifest["sourcePinsCount"] == 14, "reviewed source14 differs")
    primary = None
    try:
        checked(["git", "rev-parse", "HEAD"], "git-head-before", 15)
        require((results / "git-head-before.stdout").read_text().strip() == CASE, "candidate HEAD differs")
        checked(["git", "status", "--porcelain", "--untracked-files=no"], "git-status-before", 15)
        require(not (results / "git-status-before.stdout").read_text(), "tracked candidate already dirty")
        record["inputPinsBefore"] = []
        for pin in pins:
            path = case / pin["path"]
            require(path.resolve().is_relative_to(case) and path.is_file() and not path.is_symlink(), "candidate input is not an owned regular source")
            item = {**pin, "actualSha256": digest(path)}
            record["inputPinsBefore"].append(item)
            require(item["actualSha256"] == item["sha256"], "candidate source14 pin differs")
        for label, argv in [("rustc", ["rustc", "+stable", "--version", "--verbose"]), ("cargo", ["cargo", "+stable", "--version", "--verbose"]), ("nextest", ["cargo", "+stable", "nextest", "--version"])]:
            checked(argv, "version-" + label, 30)
        require("cargo-nextest 0.9.148" in (results / "version-nextest.stdout").read_text(), "nextest version differs")
        rustc_version = (results / "version-rustc.stdout").read_text(encoding="utf-8")
        require(re.search(r"(?m)^host: x86_64-pc-windows-msvc$", rustc_version), "rustc host must be Windows x64 MSVC")
        record["rustcHost"] = "x86_64-pc-windows-msvc"
        checked(["cargo", "+stable", "test", "--locked", "-p", "unica-coder", "--lib", "--no-run", "--message-format=json"], "build", 2400)
        messages = []
        for line in (results / "build.stdout").read_text(encoding="utf-8").splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if item.get("reason") == "compiler-artifact" and item.get("profile", {}).get("test") and item.get("executable") and Path(item["target"]["src_path"]).resolve() == case / "crates/unica-coder/src/lib.rs":
                messages.append(item)
        require(len(messages) == 1 and messages[0].get("fresh") is False and messages[0].get("features") == ["default"] and messages[0]["target"].get("kind") == ["lib"] and Path(messages[0].get("manifest_path", "")).resolve() == case / "crates/unica-coder/Cargo.toml", "no fresh case-owned libtest compiler artifact")
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
        require(len(selected) == 1 and {item["name"] for item in selected} == set(names), "actual queue list is not exact1")
        require(digest(compiled) == binary_sha and digest(retained) == binary_sha, "nextest list changed own binary")
        record["selectionActual"] = selected
        write(results / "actual-plan.json", selected)
        for number, name in enumerate(names, 1):
            require(all(digest(case / pin["path"]) == pin["sha256"] for pin in pins), "candidate source changed before first test")
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
            captured = capture_case(name, native, prefix.with_suffix(".stdout").read_text(encoding="utf-8", errors="replace"))
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
        failures, first, interrupted = finalize(case, results, pins, record, checked)
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
    record = {"status": "CAPTURE_INCOMPLETE", "plannedCases": 1, "attemptsPerCase": 1, "retries": 0, "failFast": False, "caseResults": "one first execution; setup/capture failures remain distinct"}
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
        record["plannedCasesNotStartedCount"] = 1 - len(started)
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
