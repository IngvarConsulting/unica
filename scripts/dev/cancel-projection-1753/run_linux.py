#!/usr/bin/env python3
"""#1753 exact fresh-process validation. SOURCE DRAFT: no executable CASE yet."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

OWNER_PATH = "scripts/ci/probe-unica-wire.py"
OWNER_SHA = "b126d1905a80ec5f062d79206a9f1e3ddd8ce8ce94715d8f02b45e9c789810ad"
WINDOWS_HELPER_SHA = "f995b2608359753120b1658dd0b831819ed35a3eb15f99717eda19fdcfa01fe4"

def require(ok, message):
    if not ok:
        raise RuntimeError(message)

def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()

def save(path, value):
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")

def error(exc):
    return {"type": type(exc).__name__, "message": str(exc)}

def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module

def pins_match(case, pins):
    actual = []
    for pin in pins:
        path = case / pin["path"]
        require(not path.is_symlink() and path.is_file()
                and path.resolve().is_relative_to(case), "unowned source input")
        actual.append({"path": pin["path"], "sha256": digest(path)})
        require(actual[-1]["sha256"] == pin["sha256"], "source input changed: " + pin["path"])
    return actual

def assert_no_fallback(case, environment):
    cwd = case / "crates/unica-coder"
    sentinel = "unused-existing-v5-scenario-endpoint"
    require(not os.path.lexists(cwd / sentinel), "unexpected scenario fallback in suite cwd")
    search = os.pathsep.join(str(Path(part) if Path(part).is_absolute() else cwd / part)
                            for part in environment.get("PATH", os.defpath).split(os.pathsep))
    require(shutil.which(sentinel, path=search) is None, "scenario fallback resolves through PATH")

def capture(name, pass_index, native, stdout, stderr):
    result = {"name": name, "passIndex": pass_index, "attempt": 1,
              "nativeExit": native.get("exit"), "timedOut": bool(native.get("timedOut")),
              "lifetimeConfirmed": native.get("lifetimeConfirmed"), "diagnosticComplete": False}
    result["nativeOutcome"] = ("TIMEOUT" if result["timedOut"] else
                              {0: "PASS", 101: "FAIL"}.get(result["nativeExit"], "PROCESS_ERROR"))
    try:
        require(result["nativeExit"] in [0, 101] and not result["timedOut"], "not ordinary libtest exit")
        require(len(re.findall(r"(?m)^running 1 test$", stdout)) == 1, "zero or multiple tests")
        literal = re.escape(name)
        require(len(re.findall(r"(?m)^test " + literal + r" \.\.\. (ok|FAILED)$", stdout)) == 1,
                "literal test missing or duplicated")
        summary = re.findall(r"(?m)^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;.*$", stdout)
        expected = ("ok", "1", "0", "0") if result["nativeExit"] == 0 else ("FAILED", "0", "1", "0")
        require(summary == [expected], "selected test skipped, zero selection or exit/summary mismatch")
        require(result["nativeExit"] != 0 or not any(
            marker in stdout + "\n" + stderr for marker in ("panicked at", "[1753 OLD-ORACLE")),
            "unexpected panic/old-oracle marker in successful libtest")
        require(not re.search(r"(?m)^\s*(?:TRY|FLAKY|LEAK|TIMEOUT|ABORT)\b", stdout + "\n" + stderr),
                "unexpected retry/lifetime marker")
        result["diagnosticComplete"] = True
    except BaseException as exc:
        result["captureError"] = error(exc)
    return result

def derived_junit(path, result):
    failed = result["nativeOutcome"] == "FAIL"
    broken = not result["diagnosticComplete"] or result["nativeOutcome"] not in ["PASS", "FAIL"]
    suite = ET.Element("testsuite", name="1753-direct-libtest-derived", tests="1",
                       failures=str(int(failed)), errors=str(int(broken)))
    item = ET.SubElement(suite, "testcase", classname=result["target"], name=result["name"],
                         time=str(result["durationSeconds"]))
    properties = ET.SubElement(item, "properties")
    for key in ["passIndex", "attempt", "nativeExit", "nativeOutcome", "diagnosticComplete"]:
        ET.SubElement(properties, "property", name=key, value=str(result[key]))
    if failed:
        ET.SubElement(item, "failure", message="first native libtest FAIL; see original raw streams")
    if broken:
        ET.SubElement(item, "error", message="setup/capture/cleanup is not semantic success")
    ET.SubElement(item, "system-out").text = json.dumps(result, ensure_ascii=True)
    with path.open("xb") as stream:
        ET.ElementTree(suite).write(stream, encoding="utf-8", xml_declaration=True)

def execute(args, payload, manifest, out, record, command_deadline, final_deadline):
    case = args.case.resolve()
    require(platform.system() == HOST_OS and platform.machine().lower() in ["amd64", "x86_64"],
            "wrong native host")
    require(os.environ.get("GITHUB_RUN_ATTEMPT") == "1", "only first Actions attempt")
    require(digest(case / OWNER_PATH) == OWNER_SHA, "ProcessOwnership input differs")
    owner = load("owned_1753_" + HOST_OS, case / OWNER_PATH)
    adapter = prepare_adapter(payload)
    selection = json.loads((payload / "selection.json").read_text(encoding="utf-8"))
    inputs = json.loads((payload / "input-pins.json").read_text(encoding="utf-8"))
    require(inputs["caseCommit"] == args.case_commit and inputs["ready"] is True, "CASE inputs unfrozen")
    pins = inputs["pins"]
    require(pins and len(pins) == manifest["sourcePinsCount"], "input inventory missing")
    require(len({p["path"] for p in pins}) == len(pins), "duplicate source pin")
    require({OWNER_PATH, "Cargo.toml", "Cargo.lock", "crates/unica-coder/Cargo.toml",
             ".config/nextest.toml", "scripts/ci/probe-unica-wire.py"}.issubset({p["path"] for p in pins}),
            "missing build/config/ownership inputs")
    require(selection["passesPerTest"] == 3 and selection["retries"] == 0, "wrong fixed pass schedule")
    require(selection["features"] == ["default", "receipt-ledger-test-support"], "wrong feature semantics")
    require(selection["inventoryProfile"] == "queue", "profile changed")
    tests = selection["tests"]
    require(len(tests) == 5 and len({t["name"] for t in tests}) == 5, "exact five literals required")
    require(sum(t["target"] == "lib" for t in tests) == 4
            and sum(t["target"] == "daemon_receipt_ledger" for t in tests) == 1, "wrong target matrix")
    for test in tests:
        require(re.fullmatch(r"[A-Za-z0-9_:]+", test["name"]), "unfrozen literal")
    require(manifest["environmentContractSource"] == inputs["environmentContractSource"], "environment contract differs")
    contract = inputs["environmentContractSource"]
    require(digest(case / contract["path"]) == contract["sha256"], "environment source differs")
    env = dict(os.environ)
    for key in manifest["clearEnvironment"]:
        env.pop(key, None)
    for name in ["temp", "xdg-config", "xdg-cache", "appdata", "localappdata"]:
        (out / name).mkdir()
    env.update(CARGO_TARGET_DIR=str(out / "target"), CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="1",
               RUSTUP_TOOLCHAIN="stable", CARGO_TERM_COLOR="never",
               TMPDIR=str(out / "temp"), TMP=str(out / "temp"), TEMP=str(out / "temp"),
               XDG_CONFIG_HOME=str(out / "xdg-config"), XDG_CACHE_HOME=str(out / "xdg-cache"),
               APPDATA=str(out / "appdata"), LOCALAPPDATA=str(out / "localappdata"))
    require(not os.path.lexists(out / "target"), "target must be fresh")
    record.update(caseCommit=args.case_commit, casePath=str(case), host=platform.platform(),
                  python=sys.version, explicitEnvironment={k: env[k] for k in
                  ["CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS", "RUSTUP_TOOLCHAIN"]},
                  provenance={k: os.environ.get(k) for k in ["GITHUB_SHA", "GITHUB_REF", "GITHUB_RUN_ID",
                  "GITHUB_RUN_ATTEMPT", "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion"]},
                  planned=[{"name": t["name"], "target": t["target"], "passIndex": p}
                           for t in tests for p in range(1, 4)], cases=[], artifacts=[])
    def command(argv, label, cap, cwd=case, final=False):
        cutoff = final_deadline if final else command_deadline
        remaining = cutoff - time.monotonic() - 30
        require(remaining > 0, "absolute command envelope exhausted")
        result = owned_command(argv, cwd, out / label, min(cap, remaining), env, record, owner, adapter, cutoff)
        require(time.monotonic() <= cutoff, "command exceeded original absolute cutoff")
        return result
    def checked(argv, label, cap=30, final=False):
        result = command(argv, label, cap, final=final)
        require(result.get("exit") == 0 and not result.get("timedOut"), "command failed: " + label)
        return result
    primary = None
    before = None
    try:
        checked(["git", "rev-parse", "HEAD"], "head-before", 15)
        require((out / "head-before.stdout").read_text().strip() == args.case_commit, "wrong CASE HEAD")
        checked(["git", "status", "--porcelain", "--untracked-files=no"], "status-before", 15)
        require(not (out / "status-before.stdout").read_text(), "dirty CASE")
        before = pins_match(case, pins)
        save(out / "source-before.json", before)
        for label, argv in [("rustc", ["rustc", "+stable", "--version", "--verbose"]),
                            ("cargo", ["cargo", "+stable", "--version", "--verbose"]),
                            ("nextest", ["cargo", "+stable", "nextest", "--version"])]:
            checked(argv, "version-" + label)
        require("cargo-nextest 0.9.148" in (out / "version-nextest.stdout").read_text(), "nextest pin differs")
        require(re.search(r"(?m)^host: " + re.escape(RUST_HOST) + r"$",
                          (out / "version-rustc.stdout").read_text()), "wrong rustc native host")
        target_args = ["--locked", "-p", "unica-coder", "--lib", "--test", "daemon_receipt_ledger",
                       "--features", "receipt-ledger-test-support"]
        checked(["cargo", "+stable", "test", *target_args, "--no-run", "--message-format=json"], "build", 3600)
        messages = []
        for line in (out / "build.stdout").read_text(encoding="utf-8").splitlines():
            try:
                messages.append(json.loads(line))
            except json.JSONDecodeError:
                pass
        require([m for m in messages if m.get("reason") == "build-finished"] == [
            {"reason": "build-finished", "success": True}], "no successful actual Cargo build")
        artifacts = {}
        for target, relative, kind in [
            ("lib", "crates/unica-coder/src/lib.rs", ["lib"]),
            ("daemon_receipt_ledger", "crates/unica-coder/tests/daemon_receipt_ledger.rs", ["test"])]:
            found = [m for m in messages if m.get("reason") == "compiler-artifact"
                     and m.get("profile", {}).get("test") and m.get("executable")
                     and Path(m["target"]["src_path"]).resolve() == case / relative]
            require(len(found) == 1 and found[0]["fresh"] is False
                    and found[0]["features"] == selection["features"]
                    and found[0]["target"]["kind"] == kind
                    and Path(found[0]["manifest_path"]).resolve() == case / "crates/unica-coder/Cargo.toml",
                    "wrong/missing/freshness/features compiler artifact: " + target)
            raw_path = Path(found[0]["executable"])
            require(not raw_path.is_symlink() and raw_path.is_file(), "invalid executable")
            compiled = raw_path.resolve()
            require(compiled.is_relative_to(out / "target"), "artifact outside fresh owned target")
            retained = out / ("owned-" + target + (".exe" if HOST_OS == "Windows" else ""))
            require(not os.path.lexists(retained), "retained binary already exists")
            shutil.copy2(compiled, retained)
            sha = digest(compiled)
            require(digest(retained) == sha, "copy changed")
            artifacts[target] = {"target": target, "compiled": str(compiled), "retained": str(retained),
                                 "sha256": sha, "bytes": retained.stat().st_size, "compilerMessage": found[0]}
            record["artifacts"].append(artifacts[target])
        # This selected integration calls the feature-only in-process facade.
        # A normal executable is not required and is never built separately.
        normal = [m for m in messages if m.get("reason") == "compiler-artifact"
                  and not m.get("profile", {}).get("test") and m.get("executable")
                  and m["target"].get("kind") == ["bin"]
                  and Path(m["target"]["src_path"]).resolve() == case / "crates/unica-coder/src/main.rs"]
        require(len(normal) <= 1, "ambiguous optional normal executable")
        record["integrationRoute"] = "execute_scenario_json -> feature-only in-process runtime; no CARGO_BIN_EXE in selected literal"
        record["optionalNormalArtifactEmitted"] = bool(normal)
        if normal:
            message = normal[0]
            require(message["features"] == selection["features"] and message["fresh"] is False,
                    "optional executable feature/provenance mismatch")
            raw_path = Path(message["executable"])
            require(raw_path.is_file() and not raw_path.is_symlink()
                    and raw_path.resolve().is_relative_to(out / "target"), "unowned optional executable")
            retained = out / ("owned-optional-unica" + (".exe" if HOST_OS == "Windows" else ""))
            require(not os.path.lexists(retained), "optional copy already exists")
            shutil.copy2(raw_path, retained)
            sha = digest(raw_path)
            require(digest(retained) == sha, "optional executable copy changed")
            record["artifacts"].append({"target": "optional-unica", "compiled": str(raw_path.resolve()),
                "retained": str(retained), "sha256": sha, "bytes": retained.stat().st_size,
                "compilerMessage": message, "executionBySelectedTestClaimed": False})
        expression = " | ".join("test(/^" + re.escape(t["name"]) + "$/)" for t in tests)
        checked(["cargo", "+stable", "nextest", "list", *target_args, "--profile", "queue",
                 "--message-format", "json", "-E", expression], "list", 600)
        actual = []
        suites = json.loads((out / "list.stdout").read_text(encoding="utf-8"))["rust-suites"]
        for suite in suites.values():
            for name, test in suite["testcases"].items():
                if test.get("filter-match", {}).get("status") != "matches":
                    continue
                planned = [t for t in tests if t["name"] == name]
                require(len(planned) == 1 and test.get("ignored") is False, "unrequested/ignored test")
                target = planned[0]["target"]
                artifact = artifacts[target]
                cwd = Path(suite["cwd"])
                require(suite["package-name"] == "unica-coder" and suite["status"] == "listed"
                        and suite["kind"] == ("lib" if target == "lib" else "test"),
                        "wrong suite target")
                require(Path(suite["binary-path"]).resolve() == Path(artifact["compiled"])
                        and cwd.is_absolute() and cwd == case / "crates/unica-coder"
                        and cwd.is_dir() and not cwd.is_symlink(), "binary/suite cwd mismatch")
                actual.append({"name": name, "target": target, "cwd": str(cwd), "binaryId": suite["binary-id"]})
        require(len(actual) == len(tests) and {r["name"] for r in actual} == {t["name"] for t in tests},
                "zero/missing/duplicate selection")
        save(out / "actual-selection.json", actual)
        require(pins_match(case, pins) == before, "source changed during build/list")
        assert_no_fallback(case, env)
        for artifact in record["artifacts"]:
            require(digest(artifact["compiled"]) == artifact["sha256"]
                    and digest(artifact["retained"]) == artifact["sha256"], "list changed binary")
        for test_index, test in enumerate(tests, 1):
            artifact = artifacts[test["target"]]
            suite = next(s for s in actual if s["name"] == test["name"])
            for pass_index in range(1, 4):
                assert_no_fallback(case, env)
                require(pins_match(case, pins) == before and digest(artifact["retained"]) == artifact["sha256"],
                        "source/binary changed before fresh process")
                require(all(digest(a["compiled"]) == a["sha256"] and digest(a["retained"]) == a["sha256"]
                            for a in record["artifacts"]), "artifact inventory changed before process")
                label = f"test-{test_index:02d}-pass-{pass_index}"
                started = time.monotonic()
                native = None
                caught = None
                try:
                    native = command([artifact["retained"], "--exact", test["name"], "--nocapture",
                                      "--test-threads=1"], label, 120, Path(suite["cwd"]))
                    result = capture(test["name"], pass_index, native,
                                     (out / (label + ".stdout")).read_text(encoding="utf-8", errors="replace"),
                                     (out / (label + ".stderr")).read_text(encoding="utf-8", errors="replace"))
                except BaseException as exc:
                    caught = exc
                    native = record.get("commands", [{}])[-1] if native is None else native
                    result = {"name": test["name"], "passIndex": pass_index, "attempt": 1,
                              "nativeExit": native.get("exit"), "timedOut": bool(native.get("timedOut")),
                              "nativeOutcome": ("TIMEOUT" if native.get("timedOut") else
                                {0: "PASS", 101: "FAIL"}.get(native.get("exit"), "PROCESS_ERROR")),
                              "diagnosticComplete": False, "captureError": error(exc)}
                result.update(target=test["target"], durationSeconds=time.monotonic() - started,
                              binarySha256=artifact["sha256"], commandReceipt=label + ".json")
                record["cases"].append(result)
                save(out / (label + "-outcome.json"), result)
                derived_junit(out / (label + ".junit.xml"), result)
                if result["nativeOutcome"] != "PASS" or not result["diagnosticComplete"]:
                    record["firstFailure"] = result
                    if caught is not None:
                        record.setdefault("firstError", error(caught))
                        raise caught
                    raise RuntimeError("STOP_FIRST_FAILURE: no later pass or test starts")
                require(digest(artifact["retained"]) == artifact["sha256"], "binary changed during test")
                require(all(digest(a["compiled"]) == a["sha256"] and digest(a["retained"]) == a["sha256"]
                            for a in record["artifacts"]), "artifact inventory changed during process")
        require(len(record["cases"]) == 15, "incomplete fixed process schedule")
    except BaseException as exc:
        primary = exc
        record.setdefault("firstError", error(exc))
        raise
    finally:
        errors = []
        try:
            after = pins_match(case, pins)
            save(out / "source-after.json", after)
            require(before is not None and before == after, "source-before/after mismatch")
            record["sourcePinsUnchanged"] = True
        except BaseException as exc:
            errors.append({"phase": "pins-after", **error(exc)})
        try:
            for artifact in record["artifacts"]:
                require(digest(artifact["retained"]) == artifact["sha256"]
                        and digest(artifact["compiled"]) == artifact["sha256"], "final binary changed")
            record["artifactsUnchanged"] = True
        except BaseException as exc:
            errors.append({"phase": "artifacts-after", **error(exc)})
        try:
            require(not record.get("unconfirmedCommandLifetimes"), "unknown child lifetime; no final Git command")
            checked(["git", "rev-parse", "HEAD"], "head-after", 10, final=True)
            checked(["git", "status", "--porcelain", "--untracked-files=no"], "status-after", 10, final=True)
            require((out / "head-after.stdout").read_text().strip() == args.case_commit
                    and not (out / "status-after.stdout").read_text(), "final CASE drift")
            record["headAndTrackedStateUnchanged"] = True
        except BaseException as exc:
            errors.append({"phase": "git-after", **error(exc)})
        record["finalizationErrors"] = errors
        if errors and primary is None:
            raise RuntimeError("finalization failed; see separate retained errors")


HOST_OS = "Linux"
RUST_HOST = "x86_64-unknown-linux-gnu"

def prepare_adapter(payload):
    require(all(hasattr(os, n) for n in ["waitid", "WNOWAIT", "P_PID"]), "Linux retained-leader observation unavailable")
    return None

def identity(value):
    return {key: getattr(value, key) for key in ("pid", "parent_pid", "session_id", "start_identity")}

def owned_command(argv, cwd, prefix, timeout, env, record, owner, adapter, cutoff):
    # Narrow adaptation of run_red.py: own session, absolute cutoff, TERM/KILL,
    # ordinary-file logs. WNOWAIT keeps the spawned leader unreaped until
    # session cleanup: never signal a reusable bare PGID after wait()/reap.
    row = {"argv": list(map(str, argv)), "cwd": str(cwd), "timeoutSeconds": timeout,
           "startedUnixNs": time.time_ns(), "outputTransport": "regular-files",
           "cleanupBudgetSeconds": 10, "lifetimeConfirmed": False, "signals": []}
    record["commands"].append(row)
    child = ownership = out = err = None
    primary = None
    secondary = []
    observed = None
    try:
        with prefix.with_suffix(".stdout").open("xb") as out, prefix.with_suffix(".stderr").open("xb") as err:
            child = subprocess.Popen(row["argv"], cwd=cwd, env=env, stdout=out, stderr=err, start_new_session=True)
            row.update(pid=child.pid, pgid=child.pid)
            ownership = owner.ProcessOwnership.capture(child)
            require(os.getpgid(child.pid) == child.pid and os.getsid(child.pid) == child.pid, "spawned session authority missing")
            row["initialOwnedIdentities"] = [identity(i) for i in ownership.snapshot()]
            wait_until = min(cutoff - 15, time.monotonic() + timeout)
            while time.monotonic() < wait_until:
                observed = os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                if observed is not None:
                    break
                time.sleep(min(.05, max(0, wait_until - time.monotonic())))
            row["timedOut"] = observed is None
            if observed is not None:
                row["unreapedExitObservation"] = {"pid": observed.si_pid, "code": observed.si_code, "status": observed.si_status}
    except BaseException as exc:
        primary = exc
        row["commandError"] = error(exc)
    finally:
        # Independent cleanup steps preserve the original error even if one
        # diagnostic snapshot fails. No Popen poll/wait before group signaling.
        if child is not None:
            authority = False
            try:
                witness = os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                authority = os.getpgid(child.pid) == child.pid and os.getsid(child.pid) == child.pid
                row["leaderStillOwnedBeforeCleanup"] = authority
                require(authority, "lost spawned session authority")
            except BaseException as exc:
                secondary.append({"phase": "held-leader-authority", **error(exc)})
            try:
                require(ownership is not None, "ownership capture incomplete")
                identities = ownership.snapshot()
                row["ownedBeforeCleanup"] = [identity(i) for i in identities]
                survivors = ownership.survivors(identities)
                row["normalSurvivors"] = [identity(i) for i in survivors if i.pid != child.pid]
                if observed is not None and row["normalSurvivors"]:
                    row["unexpectedNormalSurvivors"] = True
            except BaseException as exc:
                secondary.append({"phase": "pre-cleanup-snapshot", **error(exc)})
            if authority:
                for sig in (signal.SIGTERM, signal.SIGKILL):
                    try:
                        os.killpg(child.pid, sig)
                        row["signals"].append(sig.name)
                    except ProcessLookupError:
                        pass
                    except BaseException as exc:
                        secondary.append({"phase": "owned-group-signal", **error(exc)})
                    if sig == signal.SIGTERM:
                        time.sleep(.1)
            try:
                child.wait(timeout=max(.01, min(5, cutoff - time.monotonic())))
                row["exit"] = child.returncode
            except BaseException as exc:
                secondary.append({"phase": "owned-leader-reap", **error(exc)})
            try:
                require(ownership is not None, "final ownership unavailable")
                final = ownership.snapshot()
                live = ownership.survivors(final)
                row["remainingOwnedIdentities"] = [identity(i) for i in live]
                row["lifetimeConfirmed"] = not live and child.returncode is not None and not secondary
            except BaseException as exc:
                secondary.append({"phase": "final-session-observation", **error(exc)})
        row["regularLogsClosed"] = all(s is None or s.closed for s in [out, err])
        row["secondaryErrors"] = secondary
        row["finishedUnixNs"] = time.time_ns()
        for suffix in ["stdout", "stderr"]:
            try:
                row[suffix + "Sha256"] = digest(prefix.with_suffix("." + suffix))
            except BaseException as exc:
                secondary.append({"phase": suffix + "-digest", **error(exc)})
        if primary is not None:
            row["primaryError"] = error(primary)
        if not row["lifetimeConfirmed"]:
            record.setdefault("unconfirmedCommandLifetimes", []).append(str(prefix))
        save(prefix.with_suffix(".json"), row)
    if primary is not None:
        raise primary
    require(row["lifetimeConfirmed"] and row["regularLogsClosed"] and not secondary
            and not row.get("unexpectedNormalSurvivors"), "owned lifetime/capture incomplete or normal survivor")
    return row

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--case", type=Path, required=True)
    parser.add_argument("--case-commit", required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--payload-manifest-sha256", required=True)
    args = parser.parse_args()
    payload = Path(__file__).resolve().parent
    manifest = json.loads((payload / "manifest.json").read_text(encoding="utf-8"))
    require(manifest.get("ready") is True, "SOURCE DRAFT: final CASE and independent review are absent")
    require(re.fullmatch(r"[0-9a-f]{40}", args.case_commit or "")
            and manifest["caseCommit"] == args.case_commit, "unfrozen CASE")
    require(digest(payload / "manifest.json") == args.payload_manifest_sha256, "manifest changed")
    control_root = payload.parents[2]
    for member in manifest["members"]:
        path = control_root / member["path"]
        require(path.resolve().is_relative_to(control_root) and path.is_file() and not path.is_symlink()
                and digest(path) == member["sha256"], "control member differs")
    out = args.results.resolve()
    require(not os.path.lexists(out), "fresh output required; no overwrite or resume")
    out.mkdir(parents=True, exist_ok=False)
    start = time.monotonic()
    command_deadline, final_deadline = start + 5400, start + 5460
    record = {"status": "INCOMPLETE", "startedUnixNs": time.time_ns(), "startedMonotonic": start,
              "commandDeadlineMonotonic": command_deadline, "finalDeadlineMonotonic": final_deadline,
              "commandBudgetSeconds": 5400, "cleanupFinalizationSeconds": 60,
              "passesPerTest": 3, "retries": 0, "failFastWithinOS": True,
              "execution": "direct retained libtest; nextest148 inventory only; JUnit derived from native raw",
              "cases": [], "commands": [], "manifestSha256": args.payload_manifest_sha256}
    exit_code = 2
    try:
        execute(args, payload, manifest, out, record, command_deadline, final_deadline)
        record["status"] = "FIXED_TARGETS_15_FIRST_PROCESS_PASSES"
        exit_code = 0
    except BaseException as exc:
        record.setdefault("firstError", error(exc))
        record["status"] = "STOP_FIRST_FAILURE_OR_CAPTURE_INCOMPLETE"
        exit_code = 130 if isinstance(exc, KeyboardInterrupt) else 1
    finally:
        record["elapsedSeconds"] = time.monotonic() - start
        record["finishedUnixNs"] = time.time_ns()
        done = {(c["name"], c["passIndex"]) for c in record["cases"]}
        record["notRun"] = [x for x in record.get("planned", []) if (x["name"], x["passIndex"]) not in done]
        if record["elapsedSeconds"] > 5460:
            record["absoluteBudgetOverrun"] = True
            record["status"] = "INCOMPLETE_ABSOLUTE_BUDGET_OVERRUN"
            exit_code = 2
        record["exit"] = exit_code
        save(out / "receipt.json", record)
    return exit_code

if __name__ == "__main__":
    sys.exit(main())
