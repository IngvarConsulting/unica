#!/usr/bin/env python3
"""Build and run every Rust configuration of a runner in its job; verify their union."""
from __future__ import annotations

import argparse
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import re
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
LINUX = "ubuntu-latest"
HOSTS = (LINUX, "macos-14", "windows-latest")
# The queue runs the complete suite on Linux and only the platform facades on
# macOS: OS-specific code lives in the platform directories of
# INV.PLATFORM.OS-BEHIND-FACADE, and `size-filters.py` derives the
# `queue-platform` nextest profile from them. Nightly `large` and release keep
# the complete macOS matrix; pull requests do not run macOS by default.
PLATFORM_ONLY = {("queue", "macos-14")}
PLATFORM_NEXTEST_PROFILE = "queue-platform"


def platform_only(profile: str, host: str | None) -> bool:
    return (profile, host) in PLATFORM_ONLY


def lanes(profile: str, host: str | None = None) -> tuple[str, ...]:
    """Configurations one runner builds and executes for a gate profile.

    Vendor's ureq fault cases run with the complete suites only (nightly,
    release): the queue keeps ordinary and Receipt (owner's decision, #1813).
    """
    if platform_only(profile, host) or not set(ADMITTED[profile]) & {"medium", "large"}:
        return ("ordinary",)
    return LANES if "large" in ADMITTED[profile] else ("ordinary", "receipt")


def profile_hosts(profile: str, *, full_matrix: bool = True) -> list[str]:
    """Runners of a gate profile; a pull request adds macOS only with `ci:full`."""
    if profile == "large":
        return list(HOSTS)
    if profile == "pr" and not full_matrix:
        return [LINUX]
    return [LINUX, "macos-14"]


def selection(lane: str) -> tuple[str, ...]:
    return {"ordinary": ("--workspace",), "receipt": runner.LEDGER_FEATURE_SELECTION,
            "vendor": runner.UREQ_RECYCLING_SELECTION}[lane]


def shards(profile: str, lane: str, host: str | None = None) -> int:
    # One job executes every lane of its runner: a second part would only add
    # another pass over the same archive on the same machine.
    return 1


def nextest_profile(profile: str, lane: str, host: str | None) -> str:
    return PLATFORM_NEXTEST_PROFILE if platform_only(profile, host) else runner.nextest_profile(profile, selection(lane))


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


def doctest_command(root: Path = ROOT, target: str | None = None) -> list[str]:
    # Nextest neither archives nor runs doctests, and the `compile_fail` examples
    # are the guards on closed writer constructors. They run once per runner in
    # the ordinary build, against the target directory the archive just filled.
    return ["cargo", "test", "--doc", "--workspace", "--locked",
            *(["--target", target] if target is not None else []),
            "--target-dir", str(root / "target")]


DOCTEST_OUTPUT_LIMIT = 16_000
DOCTEST_CASE = {"binary": "rustdoc", "name": "workspace::doctests"}


def run_doctests(root: Path, target: str | None) -> dict:
    """The outcome travels in the lane manifest: the shard turns it into a
    report record and the union verifier refuses the gate on a failure, so a
    red doctest is not left behind in the build log alone."""
    command = doctest_command(root, target)
    # A library that fails to build reports on stderr; the trace needs it too.
    completed, seconds = timed(command, root, capture=True, merge_stderr=True)
    output = completed.stdout or ""
    print(output, end="", flush=True)
    return {"command": command, "exitCode": completed.returncode, "seconds": seconds,
            "output": output[-DOCTEST_OUTPUT_LIMIT:]}


def doctest_problem(manifest: dict) -> str | None:
    # Doctests are not platform facade tests; a platform-only host skips them.
    if manifest.get("lane") != "ordinary" or platform_only(manifest.get("profile"), manifest.get("runner")):
        return None
    outcome = manifest.get("doctests")
    if not isinstance(outcome, dict) or type(outcome.get("exitCode")) is not int:
        return "missing doctest outcome"
    if outcome["exitCode"] != 0:
        return f"workspace doctests failed with exit code {outcome['exitCode']}"
    return None


def doctest_record(manifest: dict, profile: str, host: str) -> dict:
    outcome = manifest.get("doctests") if isinstance(manifest.get("doctests"), dict) else {}
    problem = doctest_problem(manifest)
    seconds = outcome.get("seconds")
    stop = allure_results.now_ms()
    start = stop - int(seconds * 1000) if isinstance(seconds, (int, float)) and seconds >= 0 else stop
    # One synthetic case per runner: it is not in the nextest size filters,
    # and it runs in every gate that builds the ordinary lane.
    return allure_results.record(
        name=DOCTEST_CASE["name"], full_name=case_id(DOCTEST_CASE),
        status="failed" if problem else "passed", runner=host,
        labels={"language": "rust", "framework": "rustdoc", "parentSuite": "rust",
                "suite": DOCTEST_CASE["binary"], "subSuite": "workspace", "profile": profile,
                "size": "small", "rust_lane": "ordinary"},
        message=problem, trace=outcome.get("output") if problem else None, start=start, stop=stop,
    )


def reuse_command(action: str, root: Path, archive: Path, lane: str, profile: str, partition: str | None = None, *, candidate: str | None = None, host: str | None = None) -> list[str]:
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
               "--profile", nextest_profile(profile, lane, host), *selected_filters]
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


def ignored_inventory(document: dict) -> list[dict]:
    """Archive-known ignored cases, including expression/partition mismatches."""
    return sorted([
        {"binary": suite["binary-id"], "name": name, "ignored": True}
        for suite in document["rust-suites"].values()
        for name, case in suite["testcases"].items() if case.get("ignored") is True
    ], key=case_id)


def ignored_ids(manifest: dict) -> set[str]:
    catalogue = manifest.get("ignoredInventory")
    if not isinstance(catalogue, list) or any(
        not isinstance(case, dict) or case.get("ignored") is not True
        or any(not isinstance(case.get(key), str) or not case[key] for key in ("binary", "name"))
        for case in catalogue
    ):
        raise ValueError("missing or malformed archive ignored inventory")
    identifiers = [case_id(case) for case in catalogue]
    known = set(identifiers)
    if len(known) != len(identifiers) or any(
        (case_id(case) in known) != case["ignored"] for case in manifest["inventory"]
    ):
        raise ValueError("archive ignored inventory conflicts with the selected inventory")
    return known


def archive_binaries(document: dict) -> list[str]:
    """Every test binary of the archive, independent of the profile filter."""
    return sorted(suite["binary-id"] for suite in document["rust-suites"].values())


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
                    configuration={"lane": lane, "nextestProfile": nextest_profile(profile, lane, host),
                                   "selection": list(selection(lane))})
    write(path, document)


def timed(command: list[str], root: Path, *, capture: bool = False, env: dict[str, str] | None = None,
          merge_stderr: bool = False):
    print("+ " + " ".join(command), flush=True)
    start = time.monotonic()
    completed = subprocess.run(command, cwd=root, text=True, stdout=subprocess.PIPE if capture else None,
                               stderr=subprocess.STDOUT if capture and merge_stderr else None, env=env)
    return completed, time.monotonic() - start


def fixture_binary_path(identity: dict) -> str:
    target = identity.get("target")
    system = identity.get("os")
    suffix = {"Linux": "-linux-", "Darwin": "-apple-darwin", "Windows": "-windows-"}.get(system)
    if not isinstance(target, str) or re.fullmatch(r"[A-Za-z0-9_.-]+", target) is None or suffix is None or suffix not in target:
        raise ValueError("fixture binary requires the native host target")
    return f"{target}/debug/" + ("unica.exe" if system == "Windows" else "unica")


def fixture_binary_descriptor(root: Path, listed: dict, identity: dict) -> dict:
    """Bind the ordinary archive's existing non-test executable, without building."""
    metadata = listed.get("rust-build-meta", {})
    platforms = metadata.get("platforms", {})
    target = identity["target"]
    if (metadata.get("target-directory") != str(root / "target")
        or platforms.get("host", {}).get("platform", {}).get("triple") != target
        or [item.get("platform", {}).get("triple") for item in platforms.get("targets", [])] != [target]):
        raise ValueError("fixture binary metadata does not describe the native archive target")
    suite = listed.get("rust-suites", {}).get("unica-coder", {})
    if (suite.get("package-name"), suite.get("binary-id"), suite.get("kind"), suite.get("build-platform")) != ("unica-coder", "unica-coder", "lib", "target"):
        raise ValueError("fixture binary metadata is missing the unica-coder package")
    package = suite.get("package-id")
    if not isinstance(package, str) or not package:
        raise ValueError("fixture binary metadata is missing the unica-coder package identity")
    candidates = [entry for entry in metadata.get("non-test-binaries", {}).get(package, []) if entry.get("name") == "unica"]
    relative = fixture_binary_path(identity)
    described_path = candidates[0].get("path") if len(candidates) == 1 else None
    if identity["os"] == "Windows" and isinstance(described_path, str):
        described_path = described_path.replace("\\", "/")
    if len(candidates) != 1 or any(candidates[0].get(key) != value for key, value in (
        ("kind", "bin-exe"), ("build-platform", "target"),
    )) or described_path != relative:
        raise ValueError("archive must contain exactly one target unica non-test fixture binary")
    binary = root.resolve() / "target" / relative
    if not binary.is_file() or binary.resolve() != binary or binary.stat().st_size <= 0:
        raise ValueError("archive fixture binary is missing, empty or linked")
    return {"path": relative, "sha256": prepared_binary.digest(binary), "bytes": binary.stat().st_size}


def fixture_binary_environment(root: Path, document: dict) -> dict[str, str]:
    descriptor = document.get("fixtureBinary")
    if (not isinstance(descriptor, dict) or set(descriptor) != {"path", "sha256", "bytes"}
        or descriptor.get("path") != fixture_binary_path(document["identity"])
        or not isinstance(descriptor.get("sha256"), str) or re.fullmatch(r"[0-9a-f]{64}", descriptor["sha256"]) is None
        or type(descriptor.get("bytes")) is not int or descriptor["bytes"] <= 0):
        raise ValueError("missing or malformed ordinary archive fixture binary")
    # Nextest extracts the checked archive before starting tests. The parent and
    # isolated child verify these exact extracted bytes before using the fixture.
    return {**os.environ,
            "UNICA_CANONICAL_DELIVERY_SUCCESS_BINARY": str(root.resolve() / "target" / descriptor["path"]),
            "UNICA_CANONICAL_DELIVERY_SUCCESS_SHA256": descriptor["sha256"]}


# A test without a host capability (symlinks, a case-insensitive volume, ...)
# reports it through `unica_process::test_support::skip_without_host_capability`
# instead of passing silently (#1893). The helper writes one record per skip
# into `HOST_CAPABILITY_SKIPS_ENV`; the runner turns that passing case into a
# skip with the reason. A runner that must have the capability refuses the
# skip, so a lost capability on Linux or macOS is red, not a quiet skip.
HOST_CAPABILITY_SKIPS_ENV = "UNICA_HOST_CAPABILITY_SKIPS"
HOST_CAPABILITIES = frozenset({
    "symlink", "case-sensitive-filesystem", "case-insensitive-filesystem",
    "normalization-insensitive-filesystem", "enforced-permission-bits",
    "directory-mtime", "non-utf8-file-names", "rename-under-open-handle",
    "unix-process-groups", "unix-permission-bits", "windows-job-object", "windows-path-syntax",
    "archive-fixture-binary",
})
UNIX_RUNNER_CAPABILITIES = frozenset({
    "symlink", "enforced-permission-bits", "directory-mtime", "rename-under-open-handle",
    "unix-process-groups", "unix-permission-bits",
})
# The lane runner itself binds the ordinary archive's fixture binary
# (`fixture_binary_environment`), so its absence in CI is a runner defect on
# every runner; only a plain local nextest run may skip for it.
ARCHIVE_RUNNER_CAPABILITIES = frozenset({"archive-fixture-binary"})
REQUIRED_HOST_CAPABILITIES = {
    "ubuntu-latest": UNIX_RUNNER_CAPABILITIES | ARCHIVE_RUNNER_CAPABILITIES
    | {"case-sensitive-filesystem", "non-utf8-file-names"},
    "macos-14": UNIX_RUNNER_CAPABILITIES | ARCHIVE_RUNNER_CAPABILITIES
    | {"case-insensitive-filesystem", "normalization-insensitive-filesystem"},
    # A decision, not a fact about the runner: Windows stays advisory (#1523)
    # for host capabilities, so those skips are visible but none is refused yet.
    "windows-latest": ARCHIVE_RUNNER_CAPABILITIES,
}
CAPABILITY_SKIP_REASON = re.compile(r"host capability (?P<capability>[a-z0-9-]+): ")


def capability_environment(destination: Path, base: dict[str, str] | None = None) -> dict[str, str]:
    # Absolute: nextest runs each test in its own package directory, where a
    # path relative to the runner's checkout names nothing.
    directory = (destination / "capability-skips").resolve()
    directory.mkdir(parents=True, exist_ok=True)
    return {**(base if base is not None else os.environ), HOST_CAPABILITY_SKIPS_ENV: str(directory)}


def capability_skips(destination: Path) -> dict[str, tuple[int, str]]:
    """Case ID → (latest skipping attempt, reason) written by the tests of this run."""
    skips: dict[str, tuple[int, str]] = {}
    for path in sorted((destination / "capability-skips").glob("*.json")):
        record = json.loads(path.read_text(encoding="utf-8"))
        if record.get("capability") not in HOST_CAPABILITIES or not isinstance(record.get("reason"), str) \
                or not all(isinstance(record.get(key), str) and record[key] for key in ("binary", "name")) \
                or not str(record.get("attempt", "")).isdigit():
            raise ValueError(f"malformed host capability skip record {path.name}")
        identifier = record["binary"] + "::" + record["name"]
        entry = (int(record["attempt"]), f"host capability {record['capability']}: {record['reason']}")
        skips[identifier] = max(skips.get(identifier, entry), entry)
    return skips


def apply_capability_skips(tree: ET.ElementTree, skips: dict[str, tuple[int, str]], owned: set[str]) -> None:
    """Mark passing owned cases that reported a missing capability as skipped."""
    found = set()
    for suite in tree.getroot().iter("testsuite"):
        for case in suite.findall("testcase"):
            identifier = suite.get("name", "") + "::" + case.get("name", "")
            if identifier not in skips:
                continue
            if identifier not in owned:
                raise ValueError(f"host capability skip for an unassigned case {identifier}")
            found.add(identifier)
            if any(child.tag in {"failure", "error", "skipped"} for child in case):
                continue  # a failing test stays failed whatever it reported
            # Only the final, passing attempt decides: an earlier attempt that
            # skipped and then failed does not make a later pass a skip.
            attempt, message = skips[identifier]
            final = 1 + sum(child.tag in allure_results.RETRIED_ATTEMPTS for child in case)
            if attempt == final:
                ET.SubElement(case, "skipped", {"message": message})
    if set(skips) - found:
        raise ValueError("host capability skip for a case missing from JUnit: " + ", ".join(sorted(set(skips) - found)))


def capability_skip_refusal(record: dict, runner: str) -> str | None:
    """Why a skipped required case is not an allowed host capability skip."""
    found = CAPABILITY_SKIP_REASON.match(record.get("reason", ""))
    if found is None or found.group("capability") not in HOST_CAPABILITIES:
        return "a required shard test did not execute"
    if found.group("capability") in REQUIRED_HOST_CAPABILITIES.get(runner, frozenset()):
        return f"host capability {found.group('capability')} is required on {runner} but {record['id']} skipped"
    return None


def junit_outcomes(path: Path) -> list[dict]:
    records = []
    for suite in ET.parse(path).getroot().iter("testsuite"):
        for case in suite.findall("testcase"):
            markers = [child for child in case if child.tag in {"skipped", "failure", "error"}]
            skipped = case.find("skipped")
            # A retried test may skip on its final attempt for a missing host
            # capability (#1893); any other skip after an executed attempt conflicts.
            capability_skip = skipped is not None and (skipped.get("message") or "").startswith("host capability ")
            if len(markers) > 1 or (skipped is not None and not capability_skip and any(
                child.tag in allure_results.RETRIED_ATTEMPTS for child in case
            )):
                raise ValueError("JUnit case has conflicting or executed skipped outcomes")
            record = {"id": suite.get("name", "") + "::" + case.get("name", ""),
                      "status": "skipped" if skipped is not None else "failed" if markers else "passed"}
            if skipped is not None:
                record["reason"] = skipped.get("message", "")
            records.append(record)
    return records


def assigned_junit(source: Path, destination: Path, manifest: dict, plan: list[dict],
                   skips: dict[str, tuple[int, str]] | None = None) -> tuple[Path, list[dict], list[dict]]:
    """Keep raw bytes; emit only owned cases, retaining their attempt nodes."""
    destination.mkdir(parents=True, exist_ok=True)
    raw = destination / "junit.raw.xml"
    raw.write_bytes(source.read_bytes())
    observed_path = raw
    if skips:
        tree = ET.parse(raw)
        apply_capability_skips(tree, skips, {case_id(case) for case in plan})
        observed_path = destination / "junit.observed.xml"
        tree.write(observed_path, encoding="utf-8", xml_declaration=True)
    records = junit_outcomes(observed_path)
    observed = Counter(record["id"] for record in records)
    if any(count != 1 for count in observed.values()):
        raise ValueError("JUnit repeats a raw case ID")
    known_ignored = ignored_ids(manifest)
    owned = {case_id(case) for case in plan}
    annotations = [record for record in records if record["id"] not in owned]
    if any(record["id"] not in known_ignored or record["status"] != "skipped" for record in annotations):
        raise ValueError("JUnit contains an unknown or executed unassigned case")
    assigned = [record for record in records if record["id"] in owned]
    tree = ET.parse(observed_path)
    for suite in tree.getroot().iter("testsuite"):
        for case in list(suite.findall("testcase")):
            if suite.get("name", "") + "::" + case.get("name", "") not in owned:
                suite.remove(case)
        set_junit_counts(suite, suite.findall("testcase"))
    if tree.getroot().tag == "testsuites":
        set_junit_counts(tree.getroot(), list(tree.getroot().iter("testcase")))
    filtered = destination / "junit.assigned.xml"
    tree.write(filtered, encoding="utf-8", xml_declaration=True)
    return filtered, assigned, annotations


def set_junit_counts(node: ET.Element, cases: list[ET.Element]) -> None:
    node.set("tests", str(len(cases)))
    for name, tag in (("skipped", "skipped"), ("failures", "failure"), ("errors", "error")):
        node.set(name, str(sum(case.find(tag) is not None for case in cases)))


def build(root: Path, destination: Path, lane: str, profile: str, host: str, line: str | None) -> None:
    if destination.exists():
        raise ValueError(f"lane destination already exists: {destination}")
    destination.mkdir(parents=True)
    before = prepared_binary.identity(root)
    signature(destination, profile=profile, host=host, lane=lane, line=line, sha=before["sha"])
    archive = destination / "tests.tar.zst"
    completed, seconds = timed(archive_command(lane, archive, root, before["target"]), root)
    completed.check_returncode()
    if prepared_binary.identity(root) != before:
        raise ValueError("source inputs changed while building the test archive")
    doctests = run_doctests(root, before["target"]) if lane == "ordinary" and not platform_only(profile, host) else None
    full_command = reuse_command("list", root, archive, lane, profile, host=host)
    completed, list_seconds = timed(full_command, root, capture=True)
    completed.check_returncode()
    listed = json.loads(completed.stdout)
    full = inventory(listed)
    parts = []
    for index in range(1, shards(profile, lane, host) + 1):
        command = reuse_command("list", root, archive, lane, profile, f"hash:{index}/{shards(profile, lane, host)}", host=host)
        completed, _ = timed(command, root, capture=True)
        completed.check_returncode()
        parts.append(inventory(json.loads(completed.stdout)))
    validate_partition(full, parts)
    document = {
        "schemaVersion": 1, "identity": before, "workspace": str(root.resolve()),
        "runner": host, "profile": profile, "lane": lane,
        "nextestProfile": nextest_profile(profile, lane, host),
        "nextestVersion": prepared_binary.output(root, ["cargo", "nextest", "--version"]),
        "selection": list(selection(lane)), "archiveSha256": prepared_binary.digest(archive),
        "archiveBytes": archive.stat().st_size, "inventory": full, "shards": parts,
        "ignoredInventory": ignored_inventory(listed), "binaries": archive_binaries(listed),
        "buildSeconds": seconds, "listSeconds": list_seconds,
        "runId": os.environ.get("GITHUB_RUN_ID"), "attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
    }
    if lane == "ordinary":
        document["fixtureBinary"] = fixture_binary_descriptor(root, listed, before)
        if doctests is not None:
            document["doctests"] = doctests
    write(destination / "lane.json", document)
    allure_results.write_plan(destination, full)


def plan_directory(report: Path, host: str, lane: str) -> Path:
    return report / f"plan-rust-{host}-{lane}"


def result_directory(report: Path, host: str, lane: str) -> Path:
    return report / f"results-rust-{host}-{lane}-1"


def build_host(root: Path, work: Path, report: Path, profile: str, host: str, line: str | None) -> int:
    """Build every lane archive of this runner in the job that will run it.

    The archive is not handed to another job: it pins the bytes the plan lists
    to the bytes the run executes, and a later lane's build (Receipt rebuilds
    `unica` with its feature) cannot replace them, because the run extracts
    its own archive. A failed lane still leaves its signed configuration.
    """
    failed = []
    for lane in lanes(profile, host):
        try:
            build(root, work / lane, lane, profile, host, line)
        except Exception as error:  # one red lane never stops the next one
            print(f"::error::{host}/{lane}: archive was not built: {error}", flush=True)
            failed.append(lane)
        finally:
            if (work / lane).is_dir():
                destination = plan_directory(report, host, lane)
                destination.mkdir(parents=True, exist_ok=True)
                for path in (work / lane).glob("*.json"):
                    destination.joinpath(path.name).write_bytes(path.read_bytes())
    return 1 if failed else 0


def run_host(root: Path, work: Path, report: Path, profile: str, host: str, line: str | None, shadow: Path | None) -> int:
    """Run each built lane; a red lane does not stop the next one."""
    failed = []
    for lane in lanes(profile, host):
        if not (work / lane / "lane.json").is_file():
            failed.append(lane)
            continue
        try:
            code = run_archive(root, work / lane, result_directory(report, host, lane), lane, profile, host, 1, line,
                               shadow if shadow is not None and shadow.is_file() else None)
        except Exception as error:  # one red lane never stops the next one
            print(f"::error::{host}/{lane}: {error}", flush=True)
            code = 1
        if code != 0:
            failed.append(lane)
    return 1 if failed else 0


def validate_manifest(document: dict, *, profile: str, host: str, lane: str, sha: str) -> None:
    if document.get("schemaVersion") != 1 or (document.get("profile"), document.get("runner"), document.get("lane"), document.get("identity", {}).get("sha")) != (profile, host, lane, sha):
        raise ValueError("archive or result belongs to another revision, runner, lane or profile")
    if document.get("selection") != list(selection(lane)) or document.get("nextestProfile") != nextest_profile(profile, lane, host):
        raise ValueError("archive has an unexpected feature or test selection")
    if len(document.get("shards", [])) != shards(profile, lane, host):
        raise ValueError("archive has an unexpected shard count")
    validate_partition(document["inventory"], document["shards"])
    ignored_ids(document)
    binaries = document.get("binaries")
    if (not isinstance(binaries, list) or any(not isinstance(item, str) or not item for item in binaries)
        or len(set(binaries)) != len(binaries)
        or not {case["binary"] for case in document["inventory"]} <= set(binaries)):
        raise ValueError("missing or malformed archive test binary inventory")


def prepared_archive(root: Path, prepared: Path, lane: str, profile: str, host: str) -> tuple[dict, dict, Path]:
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
    return document, current, archive


def run_archive(root: Path, prepared: Path, destination: Path, lane: str, profile: str, host: str, index: int, line: str | None, shadow: Path | None = None) -> int:
    document, current, archive = prepared_archive(root, prepared, lane, profile, host)
    run_options = {"env": fixture_binary_environment(root, document)} if lane == "ordinary" else {}
    if not 1 <= index <= len(document["shards"]):
        raise ValueError("invalid shard index")
    plan = document["shards"][index - 1]
    destination.mkdir(parents=True, exist_ok=False)
    allure_results.write_plan(destination, plan)
    signature(destination, profile=profile, host=host, lane=lane, line=line, sha=current["sha"])
    if lane == "ordinary" and index == 1 and not platform_only(profile, host):
        # Written before nextest runs, so a later failure in this shard does
        # not take the doctest outcome out of the report.
        allure_results.write(destination, doctest_record(document, profile, host))
    nextest_profile = document["nextestProfile"]
    candidates = {root / "target/nextest" / nextest_profile / "junit.xml", root / "vendor/ureq/target/nextest" / nextest_profile / "junit.xml"}
    for path in candidates:
        path.unlink(missing_ok=True)
    command = reuse_command("run", root, archive, lane, profile, f"hash:{index}/{len(document['shards'])}", host=host)
    if not any(not case["ignored"] for case in plan):
        command.append("--no-tests=pass")
    completed, seconds = timed(command, root, env=capability_environment(destination, run_options.get("env")))
    found = [path for path in candidates if path.is_file()]
    records = []
    annotations = []
    if len(found) == 1:
        filtered, records, annotations = assigned_junit(found[0], destination, document, plan, capability_skips(destination))
        runner.emit_rust(destination, profile, host, filtered, selection=selection(lane))
    status = {
        "schemaVersion": 1, "sha": current["sha"], "profile": profile, "runner": host,
        "lane": lane, "shard": index, "archiveSha256": document["archiveSha256"],
        "plan": plan, "records": records, "frameworkAnnotations": annotations, "exitCode": completed.returncode,
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
            listing = reuse_command("list", root, archive, lane, profile, f"hash:{index}/{len(document['shards'])}", candidate=expression, host=host)
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
                shadow_command = reuse_command("run", root, archive, lane, profile, f"hash:{index}/{len(document['shards'])}", candidate=expression, host=host)
                shadow_run, shadow_seconds = timed(shadow_command, root, env=capability_environment(
                    destination / "shadow", run_options.get("env")))
                comparison.update(shadowNotRun=False, affectedSeconds=shadow_seconds, exitCode=shadow_run.returncode)
                shadow_reports = [path for path in candidates if path.is_file()]
                if shadow_run.returncode != 0 or len(shadow_reports) != 1:
                    write(destination / "shadow.json", comparison)
                    raise ValueError("shadow execution failed or did not report results")
                _, comparison["records"], comparison["frameworkAnnotations"] = assigned_junit(
                    shadow_reports[0], destination / "shadow", document, affected, capability_skips(destination / "shadow"))
                write(destination / "shadow.json", comparison)
                shadow_manifest = {**document, "shards": list(document["shards"])}
                shadow_manifest["shards"][index - 1] = affected
                validate_result(shadow_manifest, {**status, "plan": affected, "records": comparison["records"],
                                "frameworkAnnotations": comparison["frameworkAnnotations"], "exitCode": shadow_run.returncode}, index)
            else:
                comparison.update(kind="affected-no-runnable-tests", affectedSeconds=0, exitCode=0)
        write(destination / "shadow.json", comparison)
    return completed.returncode


# A pull request runs only small tests; medium first ran in the queue. The PR's
# ordinary archive already holds medium tests, so a Linux job runs them from
# the same bytes with the queue's nextest profile (small and medium, never
# large), narrowed to the shadow's affected packages. The queue keeps the full
# mandatory set; this is an addition, never a replacement.
MEDIUM_PROFILE = "queue"
MEDIUM_RUNNER = "ubuntu-latest"
MEDIUM_NO_TESTS = "affected packages have no tests in the ordinary archive"


def medium_selection(decision: dict | None, binaries: list[str], sha: str) -> tuple[str | None, list[str], str]:
    """Return the nextest expression (None: every medium test), packages and reason."""
    if decision is None:
        return None, [], "shadow selection unavailable; all medium tests"
    if decision.get("headSha") != sha:
        return None, [], "shadow selection belongs to another revision; all medium tests"
    try:
        expression = ci_shadow_selector.filterset(decision)
    except ValueError as error:
        return None, [], f"invalid shadow selection ({error}); all medium tests"
    if expression is None:
        return None, [], "shadow selection is full; all medium tests"
    available = {binary.split("::", 1)[0] for binary in binaries}
    packages = sorted(available.intersection(decision["packages"]))
    if not packages:
        return "", [], MEDIUM_NO_TESTS
    return ci_shadow_selector.filterset({**decision, "packages": packages}), packages, "affected packages and their reverse consumers"


def run_medium(root: Path, prepared: Path, destination: Path, host: str, shadow: Path | None) -> int:
    if host != MEDIUM_RUNNER:
        raise ValueError("medium tests of a pull request run on Linux only")
    document, current, archive = prepared_archive(root, prepared, "ordinary", "pr", host)
    decision = json.loads(shadow.read_text()) if shadow is not None and shadow.is_file() else None
    expression, packages, reason = medium_selection(decision, document["binaries"], current["sha"])
    destination.mkdir(parents=True, exist_ok=False)
    plan: list[dict] = []
    if expression != "":
        listed, _ = timed(reuse_command("list", root, archive, "ordinary", MEDIUM_PROFILE, candidate=expression), root, capture=True)
        listed.check_returncode()
        plan = inventory(json.loads(listed.stdout))
        if not plan and expression is not None:
            reason = MEDIUM_NO_TESTS  # archived binaries of these packages hold only large tests
    status = {
        "schemaVersion": 1, "sha": current["sha"], "profile": "pr", "runner": host, "lane": "ordinary",
        "shard": 1, "archiveSha256": document["archiveSha256"], "nextestProfile": MEDIUM_PROFILE,
        "expression": expression, "packages": packages, "reason": reason, "plan": plan,
        "records": [], "frameworkAnnotations": [], "exitCode": 0, "executionSeconds": 0,
        "runId": os.environ.get("GITHUB_RUN_ID"), "attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
    }
    if any(not case["ignored"] for case in plan):
        junit = root / "target/nextest" / MEDIUM_PROFILE / "junit.xml"
        junit.unlink(missing_ok=True)
        command = reuse_command("run", root, archive, "ordinary", MEDIUM_PROFILE, candidate=expression)
        completed, seconds = timed(command, root, env=capability_environment(destination, fixture_binary_environment(root, document)))
        status.update(exitCode=completed.returncode, executionSeconds=seconds)
        if junit.is_file():
            _, status["records"], status["frameworkAnnotations"] = assigned_junit(
                junit, destination, document, plan, capability_skips(destination))
    write(destination / "medium.json", status)
    validate_medium(document, status)
    return status["exitCode"]


def validate_medium(manifest: dict, medium: dict) -> None:
    if medium.get("runner") != MEDIUM_RUNNER or medium.get("nextestProfile") != MEDIUM_PROFILE:
        raise ValueError("medium tests ran on another runner or nextest profile")
    expression, packages, plan = medium.get("expression"), medium.get("packages"), medium.get("plan")
    if not isinstance(packages, list) or not isinstance(plan, list):
        raise ValueError("malformed medium selection")
    # The recorded expression must be the one the packages imply: all medium
    # (None, no packages), nothing archived (""), or exactly these packages.
    implied = None if not packages and expression is None else "" if not packages else \
        ci_shadow_selector.filterset({"mode": "affected", "mandatoryMode": "full", "affectedAdmission": False, "packages": packages})
    if expression != implied:
        raise ValueError("medium expression differs from its selected packages")
    if not plan and not (medium.get("reason") == MEDIUM_NO_TESTS and expression is not None):
        raise ValueError("medium plan is unexpectedly empty")
    # The queue profile includes the pr profile: every small case of the
    # selected packages that the pull request planned must be in the medium plan.
    selected = None if expression is None else set(packages)
    planned = {case_id(case) for case in plan}
    missing = [case_id(case) for case in manifest["inventory"]
               if (selected is None or case["binary"].split("::", 1)[0] in selected) and case_id(case) not in planned]
    if missing:
        raise ValueError(f"medium plan omits {len(missing)} planned case(s) of its packages, e.g. {missing[0]}")
    validate_result({**manifest, "shards": [plan]}, medium, 1)


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
    annotations = result.get("frameworkAnnotations", [])
    known_ignored = ignored_ids(manifest)
    if any(record["id"] in allowed or record["id"] not in known_ignored or record["status"] != "skipped" for record in annotations):
        raise ValueError("invalid unassigned ignored framework annotation")
    observed = Counter(record["id"] for record in records + annotations)
    if any(count != 1 for count in observed.values()) or not wanted.issubset({record["id"] for record in records}) or not {record["id"] for record in records}.issubset(allowed):
        raise ValueError("shard results do not cover their assigned tests exactly once")
    for record in records:
        if record["status"] == "skipped" and record["id"] in wanted:
            refusal = capability_skip_refusal(record, manifest["runner"])
            if refusal:
                raise ValueError(refusal)
    if result.get("exitCode") != 0 or any(record["status"] not in ("passed", "skipped") for record in records):
        raise ValueError("a required shard test failed")


def inventory_summary(manifests: list[dict], *, profile: str, sha: str, jobs: list[str] | None) -> dict:
    """The compact plan a later run of the same configuration compares with."""
    configurations = []
    for manifest in sorted(manifests, key=lambda item: (item["runner"], item["lane"])):
        selected = Counter(case["binary"] for case in manifest["inventory"] if not case["ignored"])
        configurations.append({
            "runner": manifest["runner"], "lane": manifest["lane"], "shards": len(manifest["shards"]),
            "binaries": sorted(manifest["binaries"]),
            "selected": {binary: selected.get(binary, 0) for binary in sorted(manifest["binaries"])},
            "cases": [case_id(case) for case in manifest["inventory"]],
        })
    return {"schemaVersion": 1, "sha": sha, "profile": profile, "runId": os.environ.get("GITHUB_RUN_ID"),
            "attempt": os.environ.get("GITHUB_RUN_ATTEMPT"), "jobs": jobs, "configurations": configurations}


def same_selection(first: str, second: str, host: str | None = None) -> bool:
    """Profiles with one size admission, lane set, nextest profile and
    partition on this host select the same plan there."""
    if first not in ADMITTED or second not in ADMITTED:
        return False
    def shape(profile):
        return (ADMITTED[profile], lanes(profile, host),
                tuple((shards(profile, lane, host), platform_only(profile, host)) for lane in lanes(profile, host)))
    return shape(first) == shape(second)


def binary_of(case: str, binaries: list[str]) -> str | None:
    # Binary ids nest (`unica-coder`, `unica-coder::daemon_receipt_ledger`):
    # a case belongs to the longest id it starts with.
    return max((binary for binary in binaries if case.startswith(binary + "::")), key=len, default=None)


def explained_case(case: str, binaries: list[str], gone: set[str], tests: list[str]) -> bool:
    """A lost case is explained by its own vanished target or a declaration naming that target."""
    binary = binary_of(case, binaries)
    if binary is None or binary in gone:
        return binary is not None
    return any(binary_of(pattern, binaries) == binary
               and (case.startswith(pattern[:-1]) if pattern.endswith("*") else case == pattern)
               for pattern in tests)


def compare_inventory(baseline: dict, current: dict) -> tuple[list[str], list[str], dict]:
    """Return blocking losses, Windows-advisory losses and the explained drift."""
    explanations = baseline.get("explanations")
    if (not isinstance(explanations, dict) or not isinstance(explanations.get("reset"), bool)
        or any(not isinstance(explanations.get(key), list)
               or any(not isinstance(item, str) or not item for item in explanations[key])
               for key in ("removedTargets", "removedTests"))):
        raise ValueError("malformed previous inventory explanations")
    previous = baseline.get("inventory")
    removed = set(explanations["removedTargets"])
    tests = list(explanations["removedTests"])
    drift = {"source": baseline.get("source"), "reset": explanations["reset"],
             "declaredBy": explanations.get("declaredBy", {}), "removedTargets": sorted(removed),
             "removedTests": sorted(tests), "removedCases": {}}
    if previous is None:
        if explanations["reset"]:
            return [], [], drift
        return ["no previous inventory of this configuration; declare 'Rust-inventory-baseline: reset' "
                "in the description of the pull request under test (for a push or tag: the one named by HEAD)"], [], drift
    if previous.get("schemaVersion") != 1 or not isinstance(previous.get("configurations"), list):
        raise ValueError("malformed previous inventory")
    problems, advisory = [], []
    # Jobs depend on the profile's runners: a pull request has no macOS job.
    same_jobs = all(same_selection(previous.get("profile"), current["profile"], host) for host in HOSTS)
    if same_jobs and previous.get("jobs") is not None and current["jobs"] is not None:
        problems += [f"job {job} disappeared since the previous inventory" for job in sorted(set(previous["jobs"]) - set(current["jobs"]))]
    present = {(item["runner"], item["lane"]): item for item in current["configurations"]}
    for old in previous["configurations"]:
        key = (old["runner"], old["lane"])
        label = f"{key[0]}/{key[1]}"
        # Comparable per host: the queue narrows macOS, not Linux.
        same_profile = same_selection(previous.get("profile"), current["profile"], key[0])
        target = advisory if current["profile"] == "large" and old["runner"] == "windows-latest" else problems
        new = present.get(key)
        if new is None:
            if same_profile or old["lane"] not in lanes(previous["profile"], key[0]):
                target.append(f"{label}: configuration and its results disappeared since the previous inventory")
            continue
        gone = (set(old["binaries"]) - set(new["binaries"])) & removed
        for binary in sorted(set(old["binaries"]) - set(new["binaries"]) - removed):
            target.append(f"{label}: test target {binary} disappeared; declare 'Removes-test-target: {binary}' "
                          "in the pull request description if intended")
        if not same_profile:
            continue
        lost = sorted(set(old["cases"]) - set(new["cases"]))
        unexplained = [case for case in lost if not explained_case(case, old["binaries"], gone, tests)]
        if lost:
            drift["removedCases"][label] = lost
        for binary, count in sorted(old["selected"].items()):
            if (count and binary in new["binaries"] and not new["selected"].get(binary)
                and any(binary_of(case, old["binaries"]) == binary for case in unexplained)):
                target.append(f"{label}: selection of test target {binary} became empty")
        if unexplained:
            shown = ", ".join(unexplained[:20]) + (f" and {len(unexplained) - 20} more" if len(unexplained) > 20 else "")
            target.append(f"{label}: {len(unexplained)} test result(s) disappeared: {shown}; "
                          "declare 'Removes-test: <binary-id>::<name or prefix*>' in the pull request description if intended")
    if explanations["reset"]:
        drift["resetSuppressed"] = problems + advisory
        return [], [], drift
    return problems, advisory, drift


def read_baseline(path: Path | None) -> dict | None:
    # A failed lookup leaves no file; CI then reports the missing comparison.
    return None if path is None or not path.is_file() else json.loads(path.read_text(encoding="utf-8"))


GATE_JOB = "Unica CI"


def os_job(host: str) -> str:
    """Display name of the job that tests one non-Linux runner."""
    return f"Rust ({host})"


def verify_union(directory: Path, profile: str, hosts: list[str], sha: str, baseline: dict | None = None) -> dict:
    if not hosts or len(hosts) != len(set(hosts)) or any(host not in HOSTS for host in hosts):
        raise ValueError("missing, duplicate or unknown OS inventory")
    manifests = [json.loads(path.read_text()) for path in directory.rglob("lane.json")]
    results = [json.loads(path.read_text()) for path in directory.rglob("shard.json")]
    problems, advisory, verified, job_names = [], [], [], None
    # The Linux gate job runs this verifier itself; the other runners' jobs
    # report their conclusions through `await_os_jobs.py`, read from the run.
    in_ci = bool(os.environ.get("CI_JOB_RESULTS"))
    unacquired: set[str] = set()
    if in_ci:
        if hosts != profile_hosts(profile) and not (profile == "pr" and hosts == [LINUX]):
            problems.append("runner matrix omits an expected operating system")
        jobs = json.loads(os.environ["CI_JOB_RESULTS"])
        job_names = sorted({GATE_JOB, *jobs})
        for host in hosts:
            if host == LINUX:
                continue
            outcome = jobs.get(os_job(host))
            if outcome != "success":
                target = advisory if profile == "large" and host == "windows-latest" else problems
                if outcome == "not_acquired":
                    # Not a lost part of the plan: GitHub had no runner of this
                    # type for the job. Its lanes are reported once, here.
                    unacquired.add(host)
                    target.append(f"{os_job(host)} got no {host} runner from GitHub (cancelled before start); "
                                  "its tests did not run, re-run the workflow")
                else:
                    target.append(f"{os_job(host)} did not finish as success: {outcome or 'missing'}")
    expected_configs = {(host, lane) for host in hosts for lane in lanes(profile, host)}
    if any((item.get("runner"), item.get("lane")) not in expected_configs for item in manifests + results):
        raise ValueError("unexpected runner or lane in the collected inventory")
    for host, lane in sorted(expected_configs):
        if host in unacquired:
            continue
        try:
            matching = [item for item in manifests if (item.get("runner"), item.get("lane")) == (host, lane)]
            if len(matching) != 1:
                raise ValueError("missing or duplicate lane plan")
            manifest = matching[0]
            validate_manifest(manifest, profile=profile, host=host, lane=lane, sha=sha)
            problem = doctest_problem(manifest)
            if problem is not None:
                # Recorded beside the shard checks, which still run: a red
                # doctest must not hide a test failure in the same lane.
                (advisory if profile == "large" and host == "windows-latest" else problems).append(f"{host}/{lane}: {problem}")
            if os.environ.get("GITHUB_RUN_ID") and any(manifest.get(key) != os.environ.get(env) for key, env in (("runId", "GITHUB_RUN_ID"), ("attempt", "GITHUB_RUN_ATTEMPT"))):
                raise ValueError("lane plan belongs to another workflow run or attempt")
            matching_results = [item for item in results if (item.get("runner"), item.get("lane")) == (host, lane)]
            if len(matching_results) != len(manifest["shards"]):
                raise ValueError("missing or duplicate shard result")
            if sorted(item.get("shard", 0) for item in matching_results) != list(range(1, len(manifest["shards"]) + 1)):
                raise ValueError("missing or duplicate shard index")
            for result in matching_results:
                validate_result(manifest, result, result["shard"])
            verified.append(manifest)
        except (KeyError, TypeError, ValueError) as error:
            # Windows large is advisory until its known failures (#1523) are fixed.
            target = advisory if profile == "large" and host == "windows-latest" else problems
            target.append(f"{host}/{lane}: {error}")
    mediums = [json.loads(path.read_text()) for path in directory.rglob("medium.json")]
    if profile == "pr" and (mediums or in_ci):
        try:
            # Any collected ordinary plan: a failed small shard must not hide the medium outcome.
            ordinary = [item for item in manifests if (item.get("runner"), item.get("lane")) == (MEDIUM_RUNNER, "ordinary")]
            if len(mediums) != 1 or len(ordinary) != 1:
                raise ValueError("missing or duplicate medium result or its ordinary plan")
            # Recompute the selection from the shadow decision this run published:
            # a wrong intersection must not pass as "no tests".
            decisions = [json.loads(path.read_text()) for path in directory.rglob("selection.json")]
            if len(decisions) > 1:
                raise ValueError("duplicate shadow selection")
            expected = medium_selection(decisions[0] if decisions else None, ordinary[0]["binaries"], sha)
            if (mediums[0].get("expression"), mediums[0].get("packages")) != expected[:2]:
                raise ValueError("medium selection differs from the shadow decision of this run")
            validate_medium(ordinary[0], mediums[0])
        except (KeyError, TypeError, ValueError) as error:
            problems.append(f"{MEDIUM_RUNNER}/ordinary medium: {error}")
    elif mediums:
        raise ValueError("medium results outside a pull request")
    summary = inventory_summary(verified, profile=profile, sha=sha, jobs=job_names)
    write(directory / "inventory.json", summary)
    drift = None
    if baseline is not None:
        lost, lost_advisory, drift = compare_inventory(baseline, summary)
        # A runner that never started loses all its configurations at once;
        # that is the one reason above, not a list of vanished lanes.
        lost = [item for item in lost if not any(item.startswith(f"{host}/") for host in unacquired)]
        lost_advisory = [item for item in lost_advisory if not any(item.startswith(f"{host}/") for host in unacquired)]
        problems += lost
        advisory += lost_advisory
    elif in_ci:
        problems.append("the plan was not compared with the previous inventory")
    report = {"sha": sha, "profile": profile, "mandatoryComplete": not problems,
              "fullMatrixPassed": not problems and not advisory, "errors": problems,
              "advisoryFailures": advisory, "affectedAdmission": False, "previousInventory": drift,
              "medium": [{key: item.get(key) for key in ("reason", "packages", "exitCode")} | {"tests": len(item.get("plan", []))}
                         for item in mediums]}
    write(directory / "verification.json", report)
    if problems:
        raise ValueError("; ".join(problems))
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "run", "medium", "verify"))
    parser.add_argument("--repo-root", type=Path, default=ROOT)
    parser.add_argument("--profile", required=True, choices=tuple(ADMITTED))
    parser.add_argument("--runner")
    parser.add_argument("--runners", default="[]")
    parser.add_argument("--destination", type=Path, help="report directory: plan-rust-*, results-rust-*, verification")
    parser.add_argument("--prepared", type=Path, help="job-local directory of the lane archives")
    parser.add_argument("--sha", default=os.environ.get("GITHUB_SHA"))
    parser.add_argument("--line")
    parser.add_argument("--shadow", type=Path)
    parser.add_argument("--baseline", type=Path, help="previous inventory found by rust_inventory_baseline.py")
    args = parser.parse_args()
    if args.action == "verify":
        print(json.dumps(verify_union(args.destination, args.profile, json.loads(args.runners), args.sha,
                                      read_baseline(args.baseline)), indent=2))
        return 0
    root = args.repo_root.resolve()
    if args.action == "build":
        return build_host(root, args.prepared, args.destination, args.profile, args.runner, args.line)
    if args.action == "medium":
        return run_medium(root, args.prepared / "ordinary", args.destination, args.runner, args.shadow)
    return run_host(root, args.prepared, args.destination, args.profile, args.runner, args.line, args.shadow)


if __name__ == "__main__":
    sys.exit(main())
