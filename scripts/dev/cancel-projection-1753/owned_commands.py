#!/usr/bin/env python3
"""Exactly one owned Windows Alpha causal observation; no automatic rerun."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time
import uuid

CASE = "de78b1f5f407179fb5eb4af63bf8e5da744dd7d7"
LOCK_SHA = "dc447d2896c7cc5c8bd0f896f3351997ebf064ffedd43289eb8f7413c686712f"
BSL_SHA = "3736453d18dd7ea12634b6a78839eed1fd3a8d821819b752b39f51afd91869d9"
PROCESS_OWNER_PATH = "scripts/ci/probe-unica-wire.py"
PROCESS_OWNER_SHA = "b126d1905a80ec5f062d79206a9f1e3ddd8ce8ce94715d8f02b45e9c789810ad"
COMMAND_CLEANUP_SECONDS = 10
TEST = "infrastructure::workspace_services::e961_one_alpha::owned_one_alpha_causal_driver"
SOURCES = {
    "crates/unica-coder/src/infrastructure/workspace_services.rs": "workspace-wire.append.rs",
    "crates/unica-coder/src/infrastructure/platform/process.rs": "process-facade.append.rs",
}


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for part in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(part)
    return result.hexdigest()


def write(path, data):
    Path(path).write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def error_record(error):
    return {"type": type(error).__name__, "message": str(error)}


def explicit_interrupt(error):
    seen = set()
    while error is not None and id(error) not in seen:
        if isinstance(error, KeyboardInterrupt):
            return error
        seen.add(id(error))
        error = error.__cause__
    return None


def load_process_owner(case, record):
    path = case / PROCESS_OWNER_PATH
    require(digest(path) == PROCESS_OWNER_SHA, "process ownership helper differs")
    spec = importlib.util.spec_from_file_location("e961_owned_command_helper", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    record["processOwnershipHelper"] = {"path": PROCESS_OWNER_PATH, "sha256": PROCESS_OWNER_SHA,
                                        "caseCommit": CASE}
    return module


def command(argv, cwd, prefix, timeout, env, owner, process_owner):
    record = {"argv": list(map(str, argv)), "cwd": str(cwd), "timeoutSeconds": timeout,
              "startedUnixNs": time.time_ns(), "cleanupBudgetSeconds": COMMAND_CLEANUP_SECONDS,
              "outputTransport": "regular-files", "lifetimeConfirmed": True}
    owner.setdefault("commands", []).append(record)
    child = ownership = out = err = None
    primary = None
    secondary = []
    interruptions = []
    def remember_interrupt(error):
        value = explicit_interrupt(error)
        if value is not None:
            interruptions.append(value)
    try:
        with prefix.with_suffix(".stdout").open("wb") as out, prefix.with_suffix(".stderr").open("wb") as err:
            # The reviewed helper assigns a suspended child to a retained
            # kill-on-close Job before it runs. No late PID/tree discovery.
            record["lifetimeConfirmed"] = False
            try:
                child = subprocess.Popen(record["argv"], cwd=cwd, env=env, stdout=out, stderr=err,
                    creationflags=getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
                                  | process_owner._WINDOWS_CREATE_SUSPENDED)
                record["pid"] = child.pid
                ownership = process_owner.ProcessOwnership.capture(
                    child, time.monotonic() + COMMAND_CLEANUP_SECONDS)
                record["jobAssignedBeforeResume"] = True
                record["exit"] = child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                record["timedOut"] = True
            except BaseException as error:
                remember_interrupt(error)
                primary = error
                record["commandError"] = error_record(error)
            finally:
                if ownership is not None:
                    try:
                        cleanup = ownership.quiesce(set(), COMMAND_CLEANUP_SECONDS)
                        record["cleanup"] = {"complete": cleanup.complete,
                            "activeProcesses": cleanup.active_processes,
                            "incomplete": list(cleanup.incomplete),
                            "survivorCount": len(cleanup.survivors)}
                        record["lifetimeConfirmed"] = (
                            cleanup.complete and cleanup.active_processes == 0
                            and child.returncode is not None)
                    except BaseException as error:
                        remember_interrupt(error)
                        secondary.append({"phase": "owned-job-cleanup", **error_record(error)})
                        if primary is None:
                            primary = error
                elif child is not None:
                    # capture() performs its own bounded rollback. If it did
                    # not return ownership, do not claim a whole-job proof.
                    try:
                        if getattr(child, "_handle", None) is not None:
                            child.kill()
                            record["fallbackLeaderExit"] = child.wait(timeout=COMMAND_CLEANUP_SECONDS)
                    except BaseException as error:
                        remember_interrupt(error)
                        secondary.append({"phase": "fallback-leader-stop", **error_record(error)})
                        if primary is None:
                            primary = error
    except BaseException as error:
        remember_interrupt(error)
        if primary is None:
            primary = error
            record["commandError"] = error_record(error)
        else:
            secondary.append({"phase": "output-log-close", **error_record(error)})
    finally:
        record["regularLogsClosed"] = all(handle is None or handle.closed for handle in [out, err])
        if not record["lifetimeConfirmed"] or not record["regularLogsClosed"]:
            owner.setdefault("unconfirmedCommandLifetimes", []).append(str(prefix))
        record["completedUnixNs"] = time.time_ns()
        for suffix in ["stdout", "stderr"]:
            try:
                path = prefix.with_suffix("." + suffix)
                if path.is_file():
                    record[suffix + "Sha256"] = digest(path)
            except BaseException as error:
                remember_interrupt(error)
                secondary.append({"phase": suffix + "-digest", **error_record(error)})
                if primary is None:
                    primary = error
        if primary is not None:
            record["primaryError"] = error_record(primary)
        if interruptions:
            record["interruptError"] = error_record(interruptions[0])
        record["secondaryErrors"] = secondary
        try:
            write(prefix.with_suffix(".json"), record)
        except BaseException as error:
            remember_interrupt(error)
            secondary.append({"phase": "command-receipt-write", **error_record(error)})
            if primary is None:
                primary = error
    if primary is not None:
        owner.setdefault("firstError", error_record(primary))
    if interruptions:
        record["interruptError"] = error_record(interruptions[0])
        if interruptions[0] is primary:
            raise primary
        raise interruptions[0] from primary
    if primary is not None:
        raise primary
    require(record["lifetimeConfirmed"] and record["regularLogsClosed"],
            "owned command lifetime is unconfirmed; preserve case and source")
    return record


def checked_command(argv, cwd, prefix, timeout, env, owner, process_owner):
    result = command(argv, cwd, prefix, timeout, env, owner, process_owner)
    require(result.get("exit") == 0 and not result.get("timedOut"), "setup/build command failed: " + str(argv))
    return result


def stage(case, root):
    spec = importlib.util.spec_from_file_location("e961_tools_builder", case / "scripts/ci/build-unica-tools.py")
    builder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(builder)
    lock_file = case / "plugins/unica/third-party/tools.lock.json"
    require(digest(lock_file) == LOCK_SHA, "tools lock differs")
    lock = builder.load_lock(lock_file)
    target = "win-x64"
    builder.assert_host(target, lock["targets"])
    cfg = lock["targets"][target]
    tool = next(item for item in lock["tools"] if item["name"] == "bsl-analyzer")
    require(tool["version"] == "0.2.86" and tool["sourceCommit"] == "53d8765288adf3b9277b7a4112dba167e51690b7", "BSL lock identity differs")
    asset = tool["assets"][target]
    require(asset["sha256"] == BSL_SHA, "BSL asset differs")
    plugin = root / "plugin"
    binary = plugin / "bin" / target / (tool["binaryName"] + cfg["exe"])
    binary.parent.mkdir(parents=True)
    (plugin / "third-party").mkdir()
    builder.download(builder.release_asset_url(tool, asset), binary, timeout=120)
    builder.verify_asset_checksum(binary, asset, tool_name=tool["name"], target=target)
    builder.set_file_mode(binary, executable=True)
    entry = builder.tool_entry(target=target, target_triple=cfg["targetTriple"], name=tool["name"],
        version=tool["version"], repository=tool["repository"], tag=tool["sourceTag"], commit=tool["sourceCommit"],
        license_id=tool["license"], binary=binary, relative_binary=binary.relative_to(plugin).as_posix(),
        delivered_binary=binary.relative_to(plugin).as_posix(), artifact=builder.artifact_name(tool))
    write(plugin / "third-party/manifest.json", {"tools": [entry], "targetTriple": cfg["targetTriple"]})
    shutil.copyfile(lock_file, plugin / "third-party/tools.lock.json")
    write(root / "engines.json", {"target": target, "tools": [entry], "selectedLockEntries": [tool], "lockSha256": LOCK_SHA})
    return plugin, binary


def atomic_replace(path, data, results):
    temporary = results / ("source-write-" + str(uuid.uuid4()) + ".rs")
    temporary.write_bytes(data)
    os.replace(temporary, path)


def finalize_sources(case, results, record, originals, instrumented, source_pins, binary):
    errors = []
    first = None

    def capture_error(phase, path, error):
        nonlocal first
        errors.append({"phase": phase, "path": str(path), **error_record(error)})
        if first is None:
            first = error

    safe = not record.get("unconfirmedCommandLifetimes")
    record["sourceRestorationDeferred"] = not safe
    restoration = []
    for path in SOURCES:
        target = case / path
        item = {"path": path, "attempted": safe, "restoredByteExact": False}
        try:
            current = target.read_bytes()
            item["sha256BeforeRestore"] = hashlib.sha256(current).hexdigest()
            if not safe:
                item["disposition"] = "preserved-unconfirmed-command-lifetime"
            elif current == instrumented[path]:
                atomic_replace(target, originals[path], results)
                item["disposition"] = "restored-known-instrumentation"
            elif current == originals[path]:
                item["disposition"] = "already-original"
            else:
                item["disposition"] = "preserved-unexpected-bytes"
                capture_error("source-restore", path, ValueError("unexpected source bytes; no overwrite"))
        except BaseException as error:
            item["restoreError"] = error_record(error)
            capture_error("source-restore", path, error)
        # Capture each path even if its restore, or the other path, failed.
        try:
            item["actualSha256"] = digest(target)
            item["restoredByteExact"] = item["actualSha256"] == hashlib.sha256(originals[path]).hexdigest()
        except BaseException as error:
            item["hashError"] = error_record(error)
            capture_error("source-after-restore-hash", path, error)
        restoration.append(item)
    record["restoration"] = restoration
    record["inputPinsAfter"] = []
    for pin in source_pins["pins"]:
        item = dict(pin)
        try:
            item["actualSha256"] = digest(case / pin["path"])
            if item["actualSha256"] != pin["sha256"]:
                capture_error("source-pin-after", pin["path"], ValueError("source/input hash differs"))
        except BaseException as error:
            item["captureError"] = error_record(error)
            capture_error("source-pin-after", pin["path"], error)
        record["inputPinsAfter"].append(item)
    record["enginePinsAfter"] = [{"name": "bsl-analyzer", "expectedSha256": BSL_SHA}]
    try:
        record["enginePinsAfter"][0]["actualSha256"] = digest(binary)
        require(record["enginePinsAfter"][0]["actualSha256"] == BSL_SHA, "native engine bytes changed")
    except BaseException as error:
        record["enginePinsAfter"][0]["captureError"] = error_record(error)
        capture_error("engine-pin-after", binary, error)
    try:
        record["processOwnershipHelperAfter"] = digest(case / PROCESS_OWNER_PATH)
        require(record["processOwnershipHelperAfter"] == PROCESS_OWNER_SHA, "process ownership helper changed")
    except BaseException as error:
        capture_error("ownership-helper-after", PROCESS_OWNER_PATH, error)
    try:
        status = subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"],
                                         cwd=case, timeout=15).decode("utf-8")
        record["trackedStatusAfter"] = status
        require(not status, "tracked source differs after restoration or is intentionally retained")
    except BaseException as error:
        capture_error("tracked-status-after", case, error)
    if not safe:
        capture_error("source-restore-deferred", case,
                      ValueError("owned command lifetime unconfirmed; instrumented case and data retained"))
    if not all(item["restoredByteExact"] for item in restoration) and first is None:
        capture_error("source-restoration", case, ValueError("source restoration incomplete"))
    record["finalizationErrors"] = errors
    if errors:
        record["captureIncomplete"] = True
    return first


def execute(args, record):
    case = args.case.resolve()
    proposal = Path(__file__).resolve().parent
    results = args.results.resolve()
    require(args.case_commit == CASE and args.target == "win-x64", "unreviewed case/target")
    require(digest(proposal / "manifest.json") == args.payload_manifest_sha256, "payload manifest differs")
    manifest = json.loads((proposal / "manifest.json").read_text(encoding="utf-8"))
    for pin in manifest["members"]:
        path = (proposal / pin["path"]).resolve()
        require(path.is_relative_to(proposal) and path.is_file() and digest(path) == pin["sha256"], "payload member differs")
    require(platform.system() == "Windows" and platform.machine().lower() in ["amd64", "x86_64"], "requires native Windows x64 runner")
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=case, text=True, timeout=15).strip() == CASE, "case checkout differs")
    require(not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=case, timeout=15), "tracked mutations before instrumentation")
    source_pins = json.loads((proposal / "source-pins.json").read_text(encoding="utf-8"))
    require(source_pins["caseCommit"] == CASE and len(source_pins["pins"]) == 16, "source16 pins differ")
    record["inputPinsBefore"] = [{**pin, "actualSha256": digest(case / pin["path"])} for pin in source_pins["pins"]]
    for pin in record["inputPinsBefore"]:
        require(pin["actualSha256"] == pin["sha256"], "source/input differs: " + pin["path"])
    wire_target = (case / "crates/unica-coder/src/infrastructure" / "../../../../../control/scripts/dev/e961-one-alpha/one_alpha.rs").resolve()
    require(wire_target == proposal / "one_alpha.rs", "control/case layout differs from reviewed static wire")
    record["controlProvenance"] = {key: os.environ.get(key) for key in ["GITHUB_SHA", "GITHUB_REF", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion"]}
    record.update(caseCommit=CASE, casePath=str(case), resultsPath=str(results), target=args.target,
        pythonVersion=sys.version, os=platform.platform(), machine=platform.machine(),
        payloadManifestSha256=args.payload_manifest_sha256,
        engineSessionsActualMeaning="session-start events after successful initialization; init-error child launch count is unproven")
    owned = results / "owned"
    owned.mkdir()
    token = str(uuid.uuid4())
    write(owned / "owner.json", {"token": token, "results": str(results)})
    (owned / "control").mkdir()
    plugin, binary = stage(case, owned)
    environment = dict(os.environ)
    contract = json.loads((case / "tests/fixtures/mcp/host-workspace-context.json").read_text(encoding="utf-8"))
    cleared = [*contract["projectEnvironment"], "UNICA_ARTIFACT_CACHE", "UNICA_RUNTIME_MANIFEST", "UNICA_HOST_CONTEXT_REQUIRED",
        "UNICA_TEST_WORKSPACE_SERVICE_EXE", "EMBEDDING_URL", "EMBEDDING_API_KEY", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTC_BOOTSTRAP", "CARGO_BUILD_TARGET",
        "UNICA_E961_CALIB_STATE", "UNICA_E961_CALIB_A", "UNICA_E961_CALIB_B", "UNICA_E961_HANDLES_STATE"]
    for name in cleared:
        environment.pop(name, None)
    environment.update(UNICA_PLUGIN_ROOT=str(plugin), UNICA_E1_COMPAT_ROOT=str(owned), UNICA_E1_COMPAT_TOKEN=token,
        UNICA_PROVIDER_STATE_DIR=str(owned / "cache/provider-state"), CARGO_TARGET_DIR=str(results / "target"), CARGO_INCREMENTAL="0",
        XDG_CONFIG_HOME=str(owned / "xdg-config"), XDG_CACHE_HOME=str(owned / "xdg-cache"),
        APPDATA=str(owned / "appdata"), LOCALAPPDATA=str(owned / "localappdata"))
    record["explicitEnvironment"] = {key: environment[key] for key in ["UNICA_PLUGIN_ROOT", "UNICA_PROVIDER_STATE_DIR", "CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "APPDATA", "LOCALAPPDATA"]}
    record["clearedEnvironment"] = cleared
    process_owner = load_process_owner(case, record)
    record["versions"] = {}
    for label, argv in [("rustc", ["rustc", "+stable", "--version", "--verbose"]), ("cargo", ["cargo", "+stable", "--version", "--verbose"]), ("bsl", [str(binary), "--version"])]:
        record["versions"][label] = checked_command(argv, owned / "control", results / ("version-" + label), 30, environment, record, process_owner)
    require("0.2.86" in (results / "version-bsl.stdout").read_text(encoding="utf-8"), "native BSL version output differs")
    originals, instrumented = {}, {}
    expected = json.loads((proposal / "instrumentation.json").read_text(encoding="utf-8"))
    for path, append_file in SOURCES.items():
        originals[path] = (case / path).read_bytes()
        instrumented[path] = originals[path] + (proposal / append_file).read_bytes()
        require(hashlib.sha256(instrumented[path]).hexdigest() == expected["instrumentedSha256"][path], "instrumented source differs from frozen expected bytes")
        label = "workspace-services" if path.endswith("workspace_services.rs") else "platform-process"
        (results / (label + ".original.rs")).write_bytes(originals[path])
        (results / (label + ".instrumented.rs")).write_bytes(instrumented[path])
    record["sourceBefore"] = [{"path": path, "sha256": hashlib.sha256(data).hexdigest()} for path, data in originals.items()]
    primary = None
    try:
        for path in SOURCES:
            atomic_replace(case / path, instrumented[path], results)
        record["build"] = checked_command(["cargo", "+stable", "test", "--locked", "-p", "unica-coder", "--lib", "--no-run", "--message-format=json", "--features", "windows-sys/Win32_System_Diagnostics_ProcessSnapshotting"], case, results / "build", 2400, environment, record, process_owner)
        artifacts = []
        with (results / "build.stdout").open(encoding="utf-8") as stream:
            for line in stream:
                try:
                    item = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if item.get("reason") == "compiler-artifact" and item.get("profile", {}).get("test") and item.get("executable") and Path(item["target"]["src_path"]).resolve() == case / "crates/unica-coder/src/lib.rs":
                    artifacts.append(item)
        require(len(artifacts) == 1 and artifacts[0].get("fresh") is False, "no fresh case-owned libtest")
        compiled = Path(artifacts[0]["executable"]).resolve()
        require(compiled.is_relative_to(results / "target"), "compiler artifact escaped isolated target")
        retained = results / "owned-libtest.exe"
        shutil.copy2(compiled, retained)
        require(digest(retained) == digest(compiled), "retained binary differs")
        record["artifact"] = {"compilerMessage": artifacts[0], "retainedPath": str(retained), "retainedSha256": digest(retained), "bytes": retained.stat().st_size}
        record["list"] = checked_command([retained, "--ignored", "--exact", TEST, "--list"], owned / "control", results / "list", 15, environment, record, process_owner)
        require((results / "list.stdout").read_text(encoding="utf-8").count(TEST + ": test") == 1, "exact one-Alpha driver not selected")
        record["observationsStarted"] = 1
        actual = command([retained, "--ignored", "--exact", TEST, "--nocapture", "--test-threads=1"], owned / "control", results / "run", 300, environment, record, process_owner)
        record["actual"] = actual
        record["observationsActual"] = 1
        require(not actual.get("timedOut"), "owned driver timed out")
        for path in SOURCES:
            require((case / path).read_bytes() == instrumented[path], "instrumented source changed: " + path)
        events = [json.loads(line) for line in (owned / "events.jsonl").read_text(encoding="utf-8").splitlines()]
        finals = [item["data"] for item in events if item["kind"] == "final"]
        drivers = [item["data"] for item in events if item["kind"] == "driver-final"]
        starts = [item["data"] for item in events if item["kind"] == "session-start"]
        cleanups = [item["data"] for item in events if item["kind"] == "session-cleanup"]
        hits = [item["data"] for item in events if item["kind"] == "semantic-hit"]
        calibrations = [item["data"] for item in events if item["kind"] == "cwd-calibration"]
        calibration_cleanups = [item["data"] for item in events if item["kind"] == "cwd-calibration-cleanup"]
        require(len(calibrations) == 1, "calibration disposition missing or duplicated")
        if calibrations[0].get("pid") is not None:
            require(len(calibration_cleanups) == 1 and calibrations[0] == calibration_cleanups[0] and calibrations[0].get("cleanupConfirmed"), "owned calibration cleanup unconfirmed")
        else:
            require(not calibration_cleanups and calibrations[0].get("childStarted") is False and calibrations[0].get("status") == "Unknown", "unknown calibration child disposition")
        require(len(finals) == 1 and len(drivers) == 1, "required final/driver evidence missing or duplicated")
        require(len(starts) <= 1 and len(hits) <= 1 and all(item.get("symbol") == "E1PinnedAlpha" for item in starts + hits), "engine/source observation expanded")
        require(len(cleanups) == len(starts) and all(item.get("confirmed") for item in cleanups), "native cleanup unconfirmed")
        driver = drivers[0]
        require(driver.get("ownedLeaderReaped") and not any(driver.get(key) for key in ["timedOut", "cancelled", "stdoutTruncated", "stderrTruncated"]), "inner driver cleanup/output incomplete")
        final = finals[0]
        record.update(final=final, engineSessionsActual=len(starts), semanticHitsActual=len(hits), eventsSha256=digest(owned / "events.jsonl"))
        validate_handle_observer(events, final, record)
        require(final.get("status") in ["PASS", "OBSERVED_FAIL"] and not final.get("captureErrors"), "forensic capture incomplete")
        if final["status"] == "PASS":
            require(actual.get("exit") == 0 and driver.get("statusSuccess") and final.get("firstFailure") is None, "PASS/exit disagreement")
        else:
            require(actual.get("exit") not in [None, 0] and not driver.get("statusSuccess") and final.get("firstFailure") is not None, "observed failure did not preserve nonzero native outcome")
        require(any(item["kind"] == "sources-finally" for item in events), "mandatory source-finally snapshot absent")
        record["observationStatus"] = final["status"]
        return 0 if final["status"] == "PASS" else 1
    except BaseException as error:
        primary = error
        record.setdefault("firstError", error_record(error))
        raise
    finally:
        finalization_error = finalize_sources(case, results, record, originals, instrumented, source_pins, binary)
        if finalization_error is not None and primary is None:
            record.setdefault("firstError", error_record(finalization_error))
            raise finalization_error



def validate_handle_observer(events, final, record):
    """One conditional, private observer; it does not replace the first rename outcome."""
    first = final.get("firstFailure") or {}
    eligible = first.get("stage") == "live-forward" and (first.get("error") or {}).get("osError") == 5
    selected = {kind: [(index, item["data"]) for index, item in enumerate(events) if item["kind"] == kind]
                for kind in ["handle-worker-start", "handle-observer-disposition", "handle-worker-cleanup", "handle-worker-response", "handle-observation"]}
    record["handleObserver"] = {"eligibleAfterFirstOS5": eligible, "counts": {key: len(value) for key, value in selected.items()}}
    if not eligible:
        require(not any(selected.values()), "observer ran outside first live OS5 boundary")
        return
    require(all(len(value) == 1 for value in selected.values()), "conditional handle observation incomplete/duplicated")
    started = selected["handle-worker-start"][0][1]
    disposition = selected["handle-observer-disposition"][0][1]
    cleanup = selected["handle-worker-cleanup"][0][1]
    response = selected["handle-worker-response"][0][1]
    observation = selected["handle-observation"][0][1]
    require(disposition == {"attempted": True, "workerQuiescent": True, "status": "Captured"}, "observer is Unknown or lifetime unconfirmed")
    require(cleanup.get("cleanupConfirmed") and cleanup.get("streamsDrained") and cleanup.get("ownedLeaderReaped") == {"Ok": True}
            and cleanup.get("statusSuccess") and cleanup.get("termination") is None, "worker did not exit normally/reap/drain")
    require(response.get("workerIdentity", {}).get("pid") == cleanup["pid"] and response.get("workerIdentity") == started.get("worker"), "worker creation identity/cleanup PID differs")
    body = response.get("result", {}).get("Ok")
    require(isinstance(body, dict) and body == observation.get("observation") and observation.get("cleanup") == cleanup
            and observation.get("firstFailureUnchanged") is True, "worker successful record disagrees with parent")
    calibration = body.get("calibration", {})
    rows = calibration.get("snapshot", {}).get("rows", [])
    require(calibration.get("status") == "Validated" and len(rows) == 4 and all(row.get("knownHandleMatched") for row in rows), "named File capability was not calibrated")
    require([value.get("rawWin32") for value in calibration.get("live", [])] == [0, 0, 32, 32]
            and [value.get("rawWin32") for value in calibration.get("afterClose", [])] == [0, 0]
            and calibration.get("allFourHoldersClosed") and calibration.get("before") == calibration.get("after"), "sharing/close/identity control differs")
    controls = calibration.get("inputControls", {})
    require(all(controls.get(key) is True for key in ["unknownNamedFileAccepted", "invalidDescriptorsRejected", "unflaggedPointerIgnored", "foreignNameNotEmitted"])
            and controls.get("win32Codes") == [{"rawWin32": 0, "classification": "opened"}, {"rawWin32": 32, "classification": "sharing_violation"}, {"rawWin32": 5, "classification": "other_win32"}], "closed input controls differ")
    require(body.get("delegatedProcessHandleClosed") and body.get("engineBefore") == body.get("engineAfter") == started.get("engine")
            and body.get("snapshot", {}).get("snapshotAndMarkerFreed") is True
            and calibration.get("snapshot", {}).get("snapshotAndMarkerFreed") is True, "process/PSS ownership not closed before engine cleanup")
    expected_paths = ["", "alpha", "alpha/Configuration.xml", "alpha/CommonModules", "alpha/CommonModules/SearchProbe.xml", "alpha/CommonModules/SearchProbe", "alpha/CommonModules/SearchProbe/Ext", "alpha/CommonModules/SearchProbe/Ext/Module.bsl", "nested", "nested/deeper"]
    require([row.get("path") for row in body.get("snapshot", {}).get("rows", [])] == expected_paths
            and [row.get("path") for row in body.get("deleteProbes", [])] == expected_paths, "observer output escaped closed fixture list")
    for row in body["deleteProbes"]:
        value = row.get("probe", {})
        code = value.get("rawWin32")
        expected = "opened" if code == 0 else "sharing_violation" if code == 32 else "other_win32"
        require(isinstance(code, int) and code >= 0 and value.get("classification") == expected
                and value.get("openedHandleClosed") is (code == 0), "probe lost its raw error or checked close")
    live = [index for index, item in enumerate(events) if item["kind"] == "rename" and item["data"].get("phase") == "live-forward"]
    retire = [index for index, item in enumerate(events) if item["kind"] == "session-cleanup"]
    post = [index for index, item in enumerate(events) if item["kind"] == "rename" and item["data"].get("phase", "").startswith("post-retire")]
    require(len(live) == 1 and len(retire) == 1 and post
            and live[0] < selected["handle-worker-start"][0][0] < selected["handle-worker-cleanup"][0][0] < selected["handle-worker-response"][0][0]
            < selected["handle-observation"][0][0] < retire[0] < min(post), "observer cleanup/engine retirement/post-rename order differs")
    record["handleObserver"].update(status="Captured", workerPID=cleanup["pid"], enginePID=body["engineBefore"]["pid"], inference="owned named-resource and DELETE-open evidence; does not identify historical rename cause")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--case", type=Path, required=True)
    parser.add_argument("--case-commit", required=True)
    parser.add_argument("--target", choices=["win-x64"], required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--payload-manifest-sha256", required=True)
    args = parser.parse_args()
    require(not os.path.lexists(args.results), "results path already exists")
    args.results.mkdir(parents=True, exist_ok=False)
    record = {"status": "CAPTURE_INCOMPLETE", "observationsPlanned": 1, "observationsActual": 0, "engineSessionsPlanned": 1}
    code = 2
    try:
        code = execute(args, record)
        record["status"] = "PASS" if code == 0 else "OBSERVED_FAIL"
    except BaseException as error:
        interrupt = explicit_interrupt(error)
        primary = interrupt if interrupt is not None else error
        if primary is not error:
            record.setdefault("secondaryErrors", []).append(
                {"phase": "explicit-interrupt-wrapper", **error_record(error)})
        record["error"] = error_record(primary)
        if interrupt is not None:
            record["firstError"] = error_record(interrupt)
        else:
            record.setdefault("firstError", error_record(primary))
        record["captureIncomplete"] = True
        record["caseAndDataRetained"] = bool(record.get("unconfirmedCommandLifetimes"))
        try:
            print("E961 first failure: " + json.dumps(record["firstError"], ensure_ascii=True), file=sys.stderr)
        except BaseException as storage_error:
            record.setdefault("secondaryStorageErrors", []).append(error_record(storage_error))
        code = 130 if interrupt is not None else 2
    finally:
        try:
            write(args.results / "receipt.json", record)
        except BaseException as error:
            try:
                print("E961 receipt write failed: " + str(error), file=sys.stderr)
            except BaseException:
                pass
            # Keep explicit interrupt disposition even if evidence storage also
            # fails. The stderr line remains the secondary capture failure.
            if isinstance(error, KeyboardInterrupt):
                code = 130
            elif code != 130:
                code = 2
    return code


def safe_main():
    try:
        return main()
    except KeyboardInterrupt:
        return 130
    except BaseException as error:
        try:
            print("E961 preflight failed: " + str(error), file=sys.stderr)
        except BaseException:
            pass
        return 130 if explicit_interrupt(error) is not None else 2


if __name__ == "__main__":
    raise SystemExit(safe_main())
