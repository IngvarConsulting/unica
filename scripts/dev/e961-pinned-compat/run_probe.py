#!/usr/bin/env python3
"""One explicit missing-platform compatibility observation. NOT EXECUTED proposal."""
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
SOURCE = "crates/unica-coder/src/infrastructure/workspace_services.rs"
PREFIX = "infrastructure::workspace_services::persistent_provider_cwd_pinned_compat::"
TEST = PREFIX + "owned_real_platform_compatibility_driver"
LOCK_SHA = "dc447d2896c7cc5c8bd0f896f3351997ebf064ffedd43289eb8f7413c686712f"


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, data):
    Path(path).write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def command(argv, cwd, prefix, timeout, env):
    record = {"argv": list(map(str, argv)), "cwd": str(cwd), "timeoutSeconds": timeout,
              "startedUnixNs": time.time_ns()}
    with prefix.with_suffix(".stdout").open("wb") as out, prefix.with_suffix(".stderr").open("wb") as err:
        child = subprocess.Popen(record["argv"], cwd=cwd, env=env, stdout=out, stderr=err)
        record["pid"] = child.pid
        try:
            record["exit"] = child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            # This only terminates the directly owned driver/build process. A
            # timeout never attains acceptance; runner disposal bounds residual
            # OS cleanup. Successful provider cleanup must come from Rust events.
            record["timedOut"] = True
            child.kill()
            record["exit"] = child.wait()
    record.update(completedUnixNs=time.time_ns(), stdoutSha256=digest(prefix.with_suffix(".stdout")),
                  stderrSha256=digest(prefix.with_suffix(".stderr")))
    write(prefix.with_suffix(".json"), record)
    return record


def stage(case, root, target):
    spec = importlib.util.spec_from_file_location("e961_tools_builder", case / "scripts/ci/build-unica-tools.py")
    builder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(builder)
    lock_file = case / "plugins/unica/third-party/tools.lock.json"
    require(digest(lock_file) == LOCK_SHA, "tools lock differs")
    lock = builder.load_lock(lock_file)
    cfg = lock["targets"][target]
    builder.assert_host(target, lock["targets"])
    names = ["rlm-bsl-mcp", "rlm-bsl-index"]
    if target == "win-x64":
        names.insert(0, "bsl-analyzer")
    selected = [next(t for t in lock["tools"] if t["name"] == name) for name in names]
    plugin = root / "plugin"
    downloads = root / "downloads"
    bin_dir = plugin / "bin" / target
    for directory in [downloads, bin_dir, plugin / "third-party"]:
        directory.mkdir(parents=True, exist_ok=False)
    # Existing archive verifier and envelope/source/target checks remain intact.
    original_download = builder.download
    builder.download = lambda url, dest, **kw: original_download(url, dest, timeout=120)
    archive_tools = [t for t in selected if t["name"] != "bsl-analyzer"]
    built, runtime_files, _ = builder.materialize_archive_group(
        archive_tools, target=target, target_triple=cfg["targetTriple"],
        downloads_dir=downloads, target_bin_dir=bin_dir, reserved_paths={})
    for tool in selected:
        if tool["name"] != "bsl-analyzer":
            continue
        asset = tool["assets"][target]
        binary = bin_dir / (tool["binaryName"] + cfg["exe"])
        builder.download(builder.release_asset_url(tool, asset), binary)
        builder.verify_asset_checksum(binary, asset, tool_name=tool["name"], target=target)
        builder.set_file_mode(binary, executable=True)
        built[tool["name"]] = binary
    entries = [builder.tool_entry(
        target=target, target_triple=cfg["targetTriple"], name=t["name"], version=t["version"],
        repository=t["repository"], tag=t["sourceTag"], commit=t["sourceCommit"], license_id=t["license"],
        binary=built[t["name"]], relative_binary=built[t["name"]].relative_to(plugin).as_posix(),
        delivered_binary=built[t["name"]].relative_to(plugin).as_posix(), artifact=builder.artifact_name(t),
    ) for t in selected]
    manifest = {"tools": entries, "targetTriple": cfg["targetTriple"]}
    write(plugin / "third-party/manifest.json", manifest)
    shutil.copyfile(lock_file, plugin / "third-party/tools.lock.json")
    evidence = {"tools": entries, "target": target, "selectedLockEntries": selected,
                "lockSha256": LOCK_SHA, "archiveRuntimeFiles": runtime_files,
                "downloadFiles": [{"name": f.name, "sha256": digest(f), "bytes": f.stat().st_size}
                                  for f in sorted(downloads.iterdir())]}
    write(root / "engines.json", evidence)
    return plugin


def execute(args, record):
    case, proposal, results = args.case.resolve(), Path(__file__).resolve().parent, args.results.resolve()
    require(args.case_commit == CASE, "unreviewed case commit")
    require(digest(proposal / "manifest.json") == args.payload_manifest_sha256, "payload manifest differs")
    for pin in json.loads((proposal / "manifest.json").read_text())["members"]:
        path = (proposal / pin["path"]).resolve()
        require(path.is_relative_to(proposal) and digest(path) == pin["sha256"], "payload member differs")
    actual_head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=case, text=True).strip()
    require(actual_head == CASE, "checkout is not the delivered commit")
    require(not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=case), "case has tracked mutations before instrumentation")
    record["controlProvenance"] = {k: os.environ.get(k) for k in ["GITHUB_SHA", "GITHUB_REF", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion"]}
    pins = json.loads((proposal / "source-pins.json").read_text())
    require(pins["caseCommit"] == CASE, "source pins name another revision")
    for pin in pins["pins"]:
        require(digest(case / pin["path"]) == pin["sha256"], "source/input bytes differ: " + pin["path"])
    require((platform.system(), args.target) in [("Linux", "linux-x64"), ("Windows", "win-x64")], "unexpected OS/target")
    require(platform.machine().lower() in ["amd64", "x86_64"], "unexpected machine")
    owned = results / "owned"
    owned.mkdir()
    token = str(uuid.uuid4())
    write(owned / "owner.json", {"token": token, "results": str(results)})
    plugin = stage(case, owned, args.target)
    (owned / "control").mkdir()
    environment = dict(os.environ)
    contract = json.loads((case / "tests/fixtures/mcp/host-workspace-context.json").read_text())
    cleared = [*contract["projectEnvironment"], "UNICA_ARTIFACT_CACHE", "UNICA_RUNTIME_MANIFEST",
               "UNICA_HOST_CONTEXT_REQUIRED", "UNICA_TEST_WORKSPACE_SERVICE_EXE", "EMBEDDING_URL", "EMBEDDING_API_KEY",
               "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTC_BOOTSTRAP", "CARGO_BUILD_TARGET"]
    for name in cleared:
        environment.pop(name, None)
    environment.update(UNICA_PLUGIN_ROOT=str(plugin), UNICA_E1_COMPAT_ROOT=str(owned),
                       UNICA_E1_COMPAT_TOKEN=token, UNICA_PROVIDER_STATE_DIR=str(owned / "cache/provider-state"),
                       CARGO_TARGET_DIR=str(results / "target"), CARGO_INCREMENTAL="0")
    # Keep source and installed runner homes independent; log only explicit paths,
    # never the full inherited environment or credentials.
    environment.update(XDG_CONFIG_HOME=str(owned / "xdg-config"), XDG_CACHE_HOME=str(owned / "xdg-cache"))
    if args.target == "win-x64":
        environment.update(APPDATA=str(owned / "appdata"), LOCALAPPDATA=str(owned / "localappdata"))
    record.update(caseCommit=CASE, target=args.target, os=platform.platform(), machine=platform.machine(),
                  explicitEnvironment={k: environment[k] for k in ["UNICA_PLUGIN_ROOT", "UNICA_PROVIDER_STATE_DIR",
                    "CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"]}, clearedEnvironment=cleared)
    source = case / SOURCE
    original = source.read_bytes()
    module = proposal / "pinned_compat.rs"
    wire = ('\n#[cfg(all(test, any(target_os = "linux", target_os = "windows")))]\n'
            f'#[path = r#"{module}"#]\nmod persistent_provider_cwd_pinned_compat;\n').encode()
    instrumented = original + wire
    (results / "original.rs").write_bytes(original)
    (results / "instrumented.rs").write_bytes(instrumented)
    source.write_bytes(instrumented)
    try:
        built = command(["cargo", "+stable", "test", "--locked", "-p", "unica-coder", "--lib", "--no-run", "--message-format=json"],
                        case, results / "build", 2400, environment)
        record["build"] = built
        require(built.get("exit") == 0 and not built.get("timedOut"), "instrumented build failed")
        artifacts = []
        for line in (results / "build.stdout").read_text(encoding="utf-8").splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if item.get("reason") == "compiler-artifact" and item.get("profile", {}).get("test") and item.get("executable") and Path(item["target"]["src_path"]).resolve() == case / "crates/unica-coder/src/lib.rs":
                artifacts.append(item)
        require(len(artifacts) == 1 and artifacts[0].get("fresh") is False, "no fresh case-owned libtest")
        artifact = artifacts[0]
        compiled = Path(artifact["executable"]).resolve()
        require(compiled.is_relative_to(results / "target"), "artifact escaped the isolated target")
        retained = results / ("owned-libtest.exe" if args.target == "win-x64" else "owned-libtest")
        shutil.copy2(compiled, retained)
        require(digest(retained) == digest(compiled), "retained binary differs")
        record["artifact"] = {"compilerMessage": artifact, "retainedSha256": digest(retained), "retainedPath": str(retained)}
        listed = command([retained, "--ignored", "--exact", TEST, "--list"], owned / "control", results / "list", 15, environment)
        require(listed.get("exit") == 0 and (results / "list.stdout").read_text().count(TEST + ": test") == 1, "exact driver not selected")
        record["observationsStarted"] = 1
        actual = command([retained, "--ignored", "--exact", TEST, "--nocapture", "--test-threads=1"],
                         owned / "control", results / "run", 300, environment)
        record["actual"] = actual
        record["observationsActual"] = 1
        require(source.read_bytes() == instrumented, "instrumented source changed during execution")
        events = [json.loads(line) for line in (owned / "events.jsonl").read_text(encoding="utf-8").splitlines()]
        final = [x["data"] for x in events if x["kind"] == "final"]
        cleanups = [x["data"] for x in events if x["kind"] == "session-cleanup"]
        drains = [x["data"] for x in events if x["kind"] == "index-cleanup"]
        driver = [x["data"] for x in events if x["kind"] == "driver-final"]
        expected = 4 if args.target == "win-x64" else 2
        observation_ok = (actual.get("exit") == 0 and not actual.get("timedOut") and
                          final == [{"status": "PASS", "error": None}] and len(cleanups) == expected and
                          all(x["confirmed"] for x in cleanups) and drains == [{"drained": True}] and
                          len(driver) == 1 and driver[0]["statusSuccess"] and driver[0]["ownedLeaderReaped"])
        record["observationStatus"] = "PASS" if observation_ok else "FAIL"
        record["eventsSha256"] = digest(owned / "events.jsonl")
        engine_plan = json.loads((owned / "engines.json").read_text())
        record["enginePinsAfter"] = [{"name": t["name"], "expectedSha256": t["sha256"],
            "actualSha256": digest(plugin / t["binaryPath"])} for t in engine_plan["tools"]]
        require(all(t["actualSha256"] == t["expectedSha256"] for t in record["enginePinsAfter"]), "engine bytes changed")
    finally:
        if source.read_bytes() == instrumented:
            source.write_bytes(original)
        record["restoredByteExact"] = source.read_bytes() == original
        record["restoredSourceSha256"] = digest(source)
        record["inputPinsAfter"] = [{**pin, "actualSha256": digest(case / pin["path"])} for pin in pins["pins"]]
        require(record["restoredByteExact"] and all(p["actualSha256"] == p["sha256"] for p in record["inputPinsAfter"]), "source/input restoration differs")
        require(not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=case), "tracked source differs after restoration")
    return 0 if record["observationStatus"] == "PASS" else 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--case", type=Path, required=True)
    parser.add_argument("--case-commit", required=True)
    parser.add_argument("--target", choices=["linux-x64", "win-x64"], required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--payload-manifest-sha256", required=True)
    args = parser.parse_args()
    args.results.mkdir(parents=True, exist_ok=False)
    record = {"status": "CAPTURE_INCOMPLETE", "observationsPlanned": 1, "observationsActual": 0}
    try:
        code = execute(args, record)
        record.update(status="PASS" if code == 0 else "OBSERVED_FAIL", observationsActual=1)
    except BaseException as error:
        code = 130 if isinstance(error, KeyboardInterrupt) else 2
        record["error"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        try:
            write(args.results / "receipt.json", record)
        except Exception:
            code = 2
    return code


def safe_main():
    try:
        return main()
    except KeyboardInterrupt:
        return 130
    except BaseException:
        return 2


if __name__ == "__main__":
    raise SystemExit(safe_main())
