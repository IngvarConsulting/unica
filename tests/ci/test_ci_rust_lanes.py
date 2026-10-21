"""Lane/shard inventory is a fail-closed partition of the original full suite."""
from __future__ import annotations
import copy
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from xml.etree import ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import ci_rust_lanes as lanes


def manifest(lane="ordinary", host="ubuntu-latest", profile="queue"):
    parts = [[{"binary": f"package-{lane}", "name": f"case_{i}", "ignored": False}] for i in range(lanes.shards(profile, lane))]
    document = {"schemaVersion": 1, "identity": {"sha": "a" * 40, "target": "x86_64-unknown-linux-gnu", "os": "Linux"}, "profile": profile, "runner": host,
            "lane": lane, "selection": list(lanes.selection(lane)), "nextestProfile": lanes.runner.nextest_profile(profile, lanes.selection(lane)),
            "archiveSha256": "archive-" + lane, "inventory": sum(parts, []), "ignoredInventory": [], "shards": parts,
            # Archive binaries independent of the profile filter; later fixtures
            # narrow the inventory to the product packages listed here.
            "binaries": sorted({f"package-{lane}", "unica-coder", "ureq"})}
    if lane == "ordinary":
        document["fixtureBinary"] = {"path": "x86_64-unknown-linux-gnu/debug/unica", "sha256": "0" * 64, "bytes": 1}
        document["doctests"] = {"command": ["cargo", "test", "--doc"], "exitCode": 0, "seconds": 0.01, "output": "ok"}
    return document


def result(plan, index):
    return {"schemaVersion": 1, "sha": plan["identity"]["sha"], "profile": plan["profile"], "runner": plan["runner"],
            "lane": plan["lane"], "shard": index, "archiveSha256": plan["archiveSha256"],
            "plan": plan["shards"][index - 1], "exitCode": 0,
            "records": [{"id": lanes.case_id(case), "status": "passed"} for case in plan["shards"][index - 1]]}


class RustLaneTests(unittest.TestCase):
    def test_archive_and_list_failures_leave_the_same_signed_configuration_before_any_plan(self):
        for stage in ("archive", "list"):
            with self.subTest(stage=stage), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                destination = root / "lane"
                identity = {"sha": "a" * 40, "target": "fixture-target"}
                calls = []

                def command(argv, _root, **kwargs):
                    calls.append(argv[2])
                    self.assertTrue((destination / "run.json").is_file(),
                                    "the archive/list starts before its signed configuration is persisted")
                    signed = json.loads((destination / "run.json").read_text())
                    self.assertEqual((signed["sha"], signed["configuration"]["lane"]),
                                     (identity["sha"], "receipt"))
                    self.assertFalse((destination / "plan.json").exists())
                    if argv[2] == stage:
                        raise subprocess.CalledProcessError(1, argv)
                    (destination / "tests.tar.zst").write_bytes(b"fixture archive")
                    return subprocess.CompletedProcess(argv, 0), 0.01

                with patch.object(lanes.prepared_binary, "identity", return_value=identity), \
                     patch.object(lanes, "timed", side_effect=command):
                    with self.assertRaises(subprocess.CalledProcessError):
                        lanes.build(root, destination, "receipt", "queue", "ubuntu-latest", "main")
                self.assertEqual(calls, ["archive"] if stage == "archive" else ["archive", "list"])
                self.assertFalse((destination / "plan.json").exists())
                self.assertFalse((destination / "lane.json").exists())

    def test_production_receipt_vendor_configs_are_separate_and_all_admitted(self):
        self.assertEqual(lanes.lanes("pr"), ("ordinary",))
        for profile in ("queue", "main", "release", "large", "all"):
            self.assertEqual(lanes.lanes(profile), ("ordinary", "receipt", "vendor"))
        self.assertNotIn("--features", lanes.archive_command("ordinary", Path("tests.tar.zst")))
        receipt = lanes.archive_command("receipt", Path("tests.tar.zst"))
        self.assertIn("receipt-ledger-test-support", receipt)
        self.assertNotIn("--workspace", receipt)
        self.assertNotIn("-E", receipt)
        self.assertIn("-E", lanes.reuse_command("run", Path("/repo"), Path("a.tar.zst"), "receipt", "queue"))
        self.assertEqual(lanes.archive_command("vendor", Path("a.tar.zst")).count("--locked"), 1)

    def test_archive_execution_does_not_accept_cargo_compile_selection(self):
        for lane in lanes.LANES:
            for action in ("list", "run"):
                command = lanes.reuse_command(action, Path("/repo"), Path("tests.tar.zst"), lane, "queue", "hash:1/2")
                self.assertIn("--archive-file", command)
                for forbidden in ("--workspace", "--features", "--lib", "--test", "--manifest-path"):
                    self.assertNotIn(forbidden, command)
                self.assertIn("hash:1/2", command)

    def test_shards_must_cover_complete_nonempty_plan_exactly_once(self):
        plan = manifest()
        lanes.validate_partition(plan["inventory"], plan["shards"])
        for parts in (plan["shards"][:-1], plan["shards"] + plan["shards"][:1], [[]], []):
            with self.subTest(parts=parts), self.assertRaises(ValueError):
                lanes.validate_partition(plan["inventory"], parts)
        changed = copy.deepcopy(plan["shards"])
        changed[0][0]["ignored"] = True
        with self.assertRaises(ValueError):
            lanes.validate_partition(plan["inventory"], changed)

    def test_stale_missing_failed_skipped_duplicate_or_wrong_feature_results_fail(self):
        plan = manifest()
        valid = result(plan, 1)
        lanes.validate_result(plan, valid, 1)
        mutations = [dict(sha="wrong"), dict(archiveSha256="receipt"), dict(records=[]), dict(exitCode=1),
                     dict(records=valid["records"] * 2), dict(records=[{**valid["records"][0], "status": "skipped"}]),
                     dict(lane="receipt"), dict(shard=2), dict(attempt="older")]
        for mutation in mutations:
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                lanes.validate_result(plan, {**valid, **mutation}, 1)

    def test_inventory_verifier_requires_each_lane_and_result_for_the_same_revision(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(lanes.os.environ, {}, clear=True):
            root = Path(temporary)
            plans = []
            for lane in lanes.LANES:
                plan = manifest(lane)
                plans.append(plan)
                lanes.write(root / lane / "lane.json", plan)
                for index in range(1, len(plan["shards"]) + 1):
                    lanes.write(root / lane / str(index) / "shard.json", result(plan, index))
            report = lanes.verify_union(root, "queue", ["ubuntu-latest"], "a" * 40)
            self.assertTrue(report["fullMatrixPassed"])
            self.assertFalse(report["affectedAdmission"])
            (root / "receipt/1/shard.json").unlink()
            with self.assertRaises(ValueError):
                lanes.verify_union(root, "queue", ["ubuntu-latest"], "a" * 40)
            self.assertFalse(json.loads((root / "verification.json").read_text())["mandatoryComplete"])

    def test_large_windows_doctest_failure_is_advisory(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(lanes.os.environ, {}, clear=True):
            root = Path(temporary)
            for lane in lanes.LANES:
                plan = manifest(lane, host="windows-latest", profile="large")
                if lane == "ordinary":
                    plan["doctests"].update(exitCode=101)
                lanes.write(root / lane / "lane.json", plan)
                for index in range(1, len(plan["shards"]) + 1):
                    lanes.write(root / lane / str(index) / "shard.json", result(plan, index))
            report = lanes.verify_union(root, "large", ["windows-latest"], "a" * 40)
            self.assertTrue(report["mandatoryComplete"])
            self.assertFalse(report["fullMatrixPassed"])
            self.assertEqual(report["advisoryFailures"],
                             ["windows-latest/ordinary: workspace doctests failed with exit code 101"])

    def test_windows_advisory_is_visible_and_never_full_matrix_passed(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(lanes.os.environ, {}, clear=True):
            report = lanes.verify_union(Path(temporary), "large", ["windows-latest"], "a" * 40)
            self.assertTrue(report["mandatoryComplete"])
            self.assertFalse(report["fullMatrixPassed"])
            self.assertEqual(len(report["advisoryFailures"]), 3)
            with self.assertRaises(ValueError):
                lanes.verify_union(Path(temporary), "large", [], "a" * 40)


class ArchiveFixtureBinaryTests(unittest.TestCase):
    def listing(self, root):
        target = "x86_64-unknown-linux-gnu"
        return {
            "rust-build-meta": {"target-directory": str(root / "target"),
                "platforms": {"host": {"platform": {"triple": target}}, "targets": [{"platform": {"triple": target}}]},
                "non-test-binaries": {"exact-coder-package": [{"name": "unica", "kind": "bin-exe", "build-platform": "target", "path": target + "/debug/unica"}]}},
            "rust-suites": {"unica-coder": {"package-name": "unica-coder", "package-id": "exact-coder-package", "kind": "lib",
                "binary-id": "unica-coder", "build-platform": "target", "testcases": {"selected": {"ignored": False}}}},
        }

    def test_native_mac_and_windows_metadata_bind_the_target_specific_executable(self):
        for system, target, name in (("Darwin", "aarch64-apple-darwin", "unica"), ("Windows", "x86_64-pc-windows-msvc", "unica.exe")):
            with self.subTest(system=system), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                listed = self.listing(root)
                metadata = listed["rust-build-meta"]
                metadata["platforms"] = {"host": {"platform": {"triple": target}}, "targets": [{"platform": {"triple": target}}]}
                relative = f"{target}/debug/{name}"
                metadata["non-test-binaries"]["exact-coder-package"][0]["path"] = relative.replace("/", "\\") if system == "Windows" else relative
                binary = root / "target" / relative
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"native archive fixture")
                descriptor = lanes.fixture_binary_descriptor(root, listed, {"target": target, "os": system})
                self.assertEqual(descriptor["path"], relative)

    def test_build_binds_only_the_existing_ordinary_archive_non_test_executable(self):
        for lane in lanes.LANES:
            with self.subTest(lane=lane), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                identity = manifest()["identity"]
                listed = self.listing(root)
                binary = root / "target/x86_64-unknown-linux-gnu/debug/unica"
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"already built non-test executable")
                calls = []

                def nextest(command, _root, *, capture=False, merge_stderr=False):
                    calls.append(command[2])
                    if command[2] == "archive":
                        (root / "prepared/tests.tar.zst").write_bytes(b"same archive")
                    return subprocess.CompletedProcess(command, 0, stdout=json.dumps(listed)), 0.01

                with patch.object(lanes.prepared_binary, "identity", return_value=identity), \
                     patch.object(lanes.prepared_binary, "output", return_value="fixture nextest"), \
                     patch.object(lanes, "signature"), patch.object(lanes, "timed", side_effect=nextest):
                    lanes.build(root, root / "prepared", lane, "pr", "ubuntu-latest", "main")
                plan = json.loads((root / "prepared/lane.json").read_text())
                self.assertEqual(calls, ["archive", *(["--doc"] if lane == "ordinary" else []), "list", "list"])
                if lane == "ordinary":
                    self.assertEqual(plan["fixtureBinary"], {"path": "x86_64-unknown-linux-gnu/debug/unica",
                        "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "bytes": binary.stat().st_size})
                else:
                    self.assertNotIn("fixtureBinary", plan)

    def test_ordinary_build_records_workspace_doctests_on_the_archive_target(self):
        # compile_fail examples guard closed writer constructors; nextest skips them.
        self.assertEqual(lanes.doctest_command(Path("/w"), "x86_64-unknown-linux-gnu"),
                         ["cargo", "test", "--doc", "--workspace", "--locked",
                          "--target", "x86_64-unknown-linux-gnu", "--target-dir", str(Path("/w") / "target")])
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            listed = ArchiveFixtureBinaryTests().listing(root)
            binary = root / "target/x86_64-unknown-linux-gnu/debug/unica"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"already built non-test executable")
            calls = []

            def command(argv, _root, *, capture=False, merge_stderr=False):
                calls.append(argv[2])
                if argv[2] == "archive":
                    (root / "prepared/tests.tar.zst").write_bytes(b"archive")
                if argv[2] == "--doc":
                    self.assertTrue(capture and merge_stderr, "the doctest output must reach the report")
                    return subprocess.CompletedProcess(argv, 101, stdout="test read::ReadFile - compile fail ... FAILED\n"), 0.01
                return subprocess.CompletedProcess(argv, 0, stdout=json.dumps(listed)), 0.01

            with patch.object(lanes.prepared_binary, "identity", return_value=manifest()["identity"]), \
                 patch.object(lanes.prepared_binary, "output", return_value="fixture nextest"), \
                 patch.object(lanes, "signature"), patch.object(lanes, "timed", side_effect=command):
                lanes.build(root, root / "prepared", "ordinary", "pr", "ubuntu-latest", "main")
            self.assertEqual(calls, ["archive", "--doc", "list", "list"])
            doctests = json.loads((root / "prepared/lane.json").read_text())["doctests"]
            self.assertEqual(doctests["exitCode"], 101)
            self.assertIn("compile fail ... FAILED", doctests["output"])

    def test_doctest_outcome_reaches_the_report_and_refuses_the_gate(self):
        passed = manifest()
        self.assertIsNone(lanes.doctest_problem(passed))
        self.assertIsNone(lanes.doctest_problem(manifest("receipt")))
        record = lanes.doctest_record(passed, "queue", "ubuntu-latest")
        self.assertEqual((record["fullName"], record["status"]), ("rustdoc::workspace::doctests", "passed"))
        self.assertIn({"name": "rust_lane", "value": "ordinary"}, record["labels"])
        self.assertIn({"name": "language", "value": "rust"}, record["labels"])
        failed = manifest()
        failed["doctests"].update(exitCode=101, output="compile fail ... FAILED")
        record = lanes.doctest_record(failed, "queue", "ubuntu-latest")
        self.assertEqual(record["status"], "failed")
        self.assertEqual(record["stop"] - record["start"], 10)
        self.assertIn("exit code 101", record["statusDetails"]["message"])
        self.assertEqual(record["statusDetails"]["trace"], "compile fail ... FAILED")
        for broken in (lambda plan: plan.pop("doctests"), lambda plan: plan.update(doctests={"exitCode": "0"})):
            plan = manifest()
            broken(plan)
            self.assertEqual(lanes.doctest_problem(plan), "missing doctest outcome")
        for outcome, mutate in (("failed", lambda plan: plan["doctests"].update(exitCode=101)),
                                ("missing", lambda plan: plan.pop("doctests"))):
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as temporary, \
                 patch.dict(lanes.os.environ, {}, clear=True):
                root = Path(temporary)
                for lane in lanes.LANES:
                    plan = manifest(lane)
                    if lane == "ordinary":
                        mutate(plan)
                    lanes.write(root / lane / "lane.json", plan)
                    for index in range(1, len(plan["shards"]) + 1):
                        if lane == "ordinary" and index == 2:
                            continue
                        lanes.write(root / lane / str(index) / "shard.json", result(plan, index))
                with self.assertRaises(ValueError):
                    lanes.verify_union(root, "queue", ["ubuntu-latest"], "a" * 40)
                errors = json.loads((root / "verification.json").read_text())["errors"]
                self.assertTrue(any("doctest" in error for error in errors), errors)
                # The shard checks of the same lane still report their own failure.
                self.assertTrue(any("shard" in error and "doctest" not in error for error in errors), errors)

    def test_missing_duplicate_wrong_package_kind_platform_path_and_link_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            original = self.listing(root)
            identity = manifest()["identity"]
            binary = root / "target/x86_64-unknown-linux-gnu/debug/unica"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"archive binary")
            mutations = [
                lambda row: row["rust-build-meta"].pop("non-test-binaries"),
                lambda row: row["rust-suites"]["unica-coder"].update({"package-id": "another-package"}),
                lambda row: row["rust-suites"]["unica-coder"].update({"package-name": "another-package"}),
                lambda row: row["rust-build-meta"]["platforms"]["host"]["platform"].update({"triple": "another-host"}),
                lambda row: row["rust-build-meta"]["platforms"]["targets"].append({"platform": {"triple": "another-target"}}),
                lambda row: row["rust-build-meta"].update({"target-directory": str(root / "elsewhere")}),
            ]
            for changes in ({"kind": "bin"}, {"build-platform": "host"}, {"name": "unica-bootstrap"},
                            {"path": "../unica"}, {"path": "/tmp/unica"}, {"path": "x86_64-unknown-linux-gnu/debug/deps/unica"}):
                mutations.append(lambda row, changes=changes: row["rust-build-meta"]["non-test-binaries"]["exact-coder-package"][0].update(changes))
            mutations.append(lambda row: row["rust-build-meta"]["non-test-binaries"]["exact-coder-package"].append(
                copy.deepcopy(row["rust-build-meta"]["non-test-binaries"]["exact-coder-package"][0])))
            for index, mutate in enumerate(mutations):
                row = copy.deepcopy(original)
                mutate(row)
                with self.subTest(mutation=index), self.assertRaises(ValueError):
                    lanes.fixture_binary_descriptor(root, row, identity)
            binary.unlink()
            with self.assertRaises(ValueError):
                lanes.fixture_binary_descriptor(root, original, identity)
            other = root / "unrelated"
            other.write_bytes(b"unrelated")
            binary.symlink_to(other)
            with self.assertRaises(ValueError):
                lanes.fixture_binary_descriptor(root, original, identity)

    def run_fixture(self, lane="ordinary", mutate=None, *, shadow=False, inspect=None):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(lanes.os.environ, {}, clear=True):
            root = Path(temporary).resolve()
            prepared = root / "prepared"
            archive = prepared / "tests.tar.zst"
            prepared.mkdir()
            archive.write_bytes(b"bound archive")
            plan = manifest(lane, profile="pr")
            plan.update(workspace=str(root), nextestVersion="fixture nextest",
                        archiveSha256=lanes.prepared_binary.digest(archive), archiveBytes=archive.stat().st_size)
            payload = b"archive's existing non-test product"
            if lane == "ordinary":
                plan["fixtureBinary"].update(sha256=hashlib.sha256(payload).hexdigest(), bytes=len(payload))
            if mutate:
                mutate(plan)
            lanes.write(prepared / "lane.json", plan)
            decision = root / "decision.json" if shadow else None
            if decision:
                lanes.write(decision, {"mode": "affected", "mandatoryMode": "full", "affectedAdmission": False,
                    "headSha": plan["identity"]["sha"], "packages": ["package-" + lane]})
            case = plan["inventory"][0]
            calls = []

            def nextest(command, **kwargs):
                environment = kwargs["env"]
                if command[2] == "run":
                    # Every execution gives tests a directory for host capability skips (#1893).
                    self.assertTrue(Path(environment.pop(lanes.HOST_CAPABILITY_SKIPS_ENV)).is_dir())
                calls.append((command[2], environment))
                if command[2] == "list":
                    return subprocess.CompletedProcess(command, 0, stdout=json.dumps({"rust-suites": {case["binary"]: {
                        "binary-id": case["binary"], "testcases": {case["name"]: {"ignored": False}}}}}))
                if lane == "ordinary":
                    path = Path(kwargs["env"]["UNICA_CANONICAL_DELIVERY_SUCCESS_BINARY"])
                    self.assertEqual(path, root / "target/x86_64-unknown-linux-gnu/debug/unica")
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(payload)
                    self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), kwargs["env"]["UNICA_CANONICAL_DELIVERY_SUCCESS_SHA256"])
                else:
                    self.assertNotIn("UNICA_CANONICAL_DELIVERY_SUCCESS_BINARY", kwargs["env"] or {})
                junit = root / "target/nextest" / plan["nextestProfile"] / "junit.xml"
                junit.parent.mkdir(parents=True, exist_ok=True)
                junit.write_text(f'<testsuites><testsuite name="{case["binary"]}"><testcase name="{case["name"]}"/></testsuite></testsuites>')
                return subprocess.CompletedProcess(command, 0)

            with patch.object(lanes.prepared_binary, "identity", return_value=plan["identity"]), \
                 patch.object(lanes.prepared_binary, "output", return_value=plan["nextestVersion"]), \
                 patch.object(lanes, "signature"), patch.object(lanes.runner, "emit_rust"), \
                 patch.object(lanes.subprocess, "run", side_effect=nextest) as executed:
                if mutate:
                    with self.assertRaises(ValueError):
                        lanes.run_archive(root, prepared, root / "result", lane, "pr", "ubuntu-latest", 1, None)
                    executed.assert_not_called()
                else:
                    self.assertEqual(lanes.run_archive(root, prepared, root / "result", lane, "pr", "ubuntu-latest", 1, None, decision), 0)
            if inspect:
                inspect(root / "result")
            return calls

    def test_actual_timed_full_and_shadow_use_the_same_binding_while_other_lanes_are_unchanged(self):
        calls = self.run_fixture(shadow=True)
        self.assertEqual([kind for kind, _ in calls], ["run", "list", "run"])
        self.assertEqual(calls[0][1], calls[2][1])
        self.assertIsNone(calls[1][1], "listing does not receive fixture execution inputs")
        for lane in ("receipt", "vendor"):
            self.assertEqual(self.run_fixture(lane), [("run", {})])

    def test_first_ordinary_shard_reports_the_doctest_outcome(self):
        def records(directory):
            return [json.loads(path.read_text()) for path in directory.glob("*-result.json")]
        seen = {}
        self.run_fixture(inspect=lambda directory: seen.update(ordinary=records(directory)))
        self.assertEqual([(entry["fullName"], entry["status"]) for entry in seen["ordinary"]],
                         [("rustdoc::workspace::doctests", "passed")])
        for lane in ("receipt", "vendor"):
            self.run_fixture(lane, inspect=lambda directory, lane=lane: seen.update({lane: records(directory)}))
            self.assertEqual(seen[lane], [])

    def test_missing_half_malformed_escaping_and_wrong_target_descriptors_stop_before_execution(self):
        self.run_fixture(mutate=lambda plan: plan.pop("fixtureBinary"))
        for descriptor in (None, {}, {"path": "x86_64-unknown-linux-gnu/debug/unica"},
                           {"sha256": "f" * 64, "bytes": 1}):
            with self.subTest(descriptor=descriptor):
                self.run_fixture(mutate=lambda plan, descriptor=descriptor: plan.update(fixtureBinary=descriptor))
        for change in ({"path": "../unica"}, {"path": "/tmp/unica"}, {"path": "other-target/debug/unica"},
                       {"path": "x86_64-unknown-linux-gnu/debug/unica.exe"}, {"sha256": "not-a-digest"},
                       {"sha256": "F" * 64}, {"bytes": 0}, {"bytes": True}, {"extra": "unbound"}):
            with self.subTest(change=change):
                self.run_fixture(mutate=lambda plan, change=change: plan["fixtureBinary"].update(change))


class IgnoredJUnitArchiveTests(unittest.TestCase):
    # Exact names/reason from root's native nightly-full/hash-partition control.
    native_ignored = (
        "infrastructure::internal_adapters::tests::system_process_runner_large_stderr_helper",
        "infrastructure::internal_adapters::tests::system_process_runner_large_stdout_helper",
        "infrastructure::workspace_services::persistent_provider_cwd_native_tests::helper_inherited_cwd_subprocess_fixture",
    )

    def run_archive_with_ignored(self, lane="receipt", *, change=None):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(lanes.os.environ, {}, clear=True):
            root = Path(temporary)
            prepared = root / "prepared"
            prepared.mkdir()
            archive = prepared / "tests.tar.zst"
            archive.write_bytes(b"immutable archive fixture")
            selected = {"binary": "unica-coder", "name": "selected", "ignored": False}
            ignored = [{"binary": "unica-coder", "name": name, "ignored": True} for name in self.native_ignored]
            plan = manifest(lane, profile="large")
            plan.update(workspace=str(root.resolve()), nextestVersion="fixture nextest", inventory=[selected],
                        ignoredInventory=ignored, shards=[[selected]],
                        archiveSha256=lanes.prepared_binary.digest(archive), archiveBytes=archive.stat().st_size)
            if lane == "ordinary":
                plan.update(inventory=[selected, *ignored], shards=[[selected], ignored])
            raw_cases = [(selected["name"], "passed"), *[(case["name"], "skipped") for case in ignored]]
            if change is not None:
                change(plan, raw_cases)
            lanes.write(prepared / "lane.json", plan)
            skipped = '<skipped message="Skipped: test does not match the run-ignored option"/>'
            markers = {"passed": "", "skipped": skipped, "failed": '<failure message="failed"/>',
                       "conflict": skipped + '<failure message="failed"/>',
                       "attempted_skip": skipped + '<rerunFailure message="earlier failure"/>'}
            raw = ('<testsuites tests="4" skipped="3"><testsuite name="unica-coder" tests="4" skipped="3">'
                   + ''.join(f'<testcase name="{name}">{markers[status]}</testcase>'
                             for name, status in raw_cases) + '</testsuite></testsuites>').encode()

            def nextest(command, _root, *, capture=False, env=None):
                self.assertEqual(command[2], "run")
                junit = root / "target/nextest" / plan["nextestProfile"] / "junit.xml"
                junit.parent.mkdir(parents=True, exist_ok=True)
                junit.write_bytes(raw)
                return subprocess.CompletedProcess(command, 0), 0.2

            destination = root / "result"
            with patch.object(lanes.prepared_binary, "identity", return_value=plan["identity"]), \
                 patch.object(lanes.prepared_binary, "output", return_value=plan["nextestVersion"]), \
                 patch.object(lanes, "signature"), patch.object(lanes, "timed", side_effect=nextest), \
                 patch.object(lanes.runner.allure_results, "ignore_reasons", return_value={}):
                try:
                    status = lanes.run_archive(root, prepared, destination, lane, "large", "ubuntu-latest", 1, None)
                except ValueError:
                    if (root / "target/nextest" / plan["nextestProfile"] / "junit.xml").exists():
                        self.assertEqual((destination / "junit.raw.xml").read_bytes(), raw)
                    raise
            record = json.loads((destination / "shard.json").read_text())
            allure = [json.loads(path.read_text()) for path in destination.glob("*-result.json")]
            return status, record, raw, (destination / "junit.raw.xml").read_bytes(), allure

    def test_native_ignored_outside_receipt_filter_stays_visible_without_allure_duplicates(self):
        status, record, original, copied, allure = self.run_archive_with_ignored()
        self.assertEqual(status, 0)
        self.assertEqual(original, copied, "the exact native-shaped JUnit remains available")
        self.assertEqual(record["records"], [{"id": "unica-coder::selected", "status": "passed"}])
        self.assertEqual({row["id"] for row in record["frameworkAnnotations"]},
                         {"unica-coder::" + name for name in self.native_ignored})
        self.assertTrue(all(row["status"] == "skipped" for row in record["frameworkAnnotations"]))
        self.assertEqual([entry["fullName"] for entry in allure], ["unica-coder::selected"])

    def test_other_partition_ignored_cases_are_annotations_not_duplicate_results(self):
        _, record, original, copied, allure = self.run_archive_with_ignored("ordinary")
        self.assertEqual(original, copied)
        self.assertEqual(len(record["frameworkAnnotations"]), 3)
        self.assertEqual([row["id"] for row in record["records"]], ["unica-coder::selected"])
        # The ordinary lane adds its one doctest record beside the single nextest record.
        self.assertEqual(sorted(entry["fullName"] for entry in allure),
                         ["rustdoc::workspace::doctests", "unica-coder::selected"])

    def test_raw_listing_retains_ignored_mismatches_separately_from_selected_plan(self):
        listing = {"rust-suites": {"bin": {"binary-id": "unica-coder", "testcases": {
            "selected": {"ignored": False, "filter-match": {"status": "matches"}},
            "outside_ignored": {"ignored": True, "filter-match": {"status": "mismatch", "reason": "expression"}},
            "outside_required": {"ignored": False, "filter-match": {"status": "mismatch", "reason": "partition"}},
        }}}}
        self.assertEqual(lanes.inventory(listing), [{"binary": "unica-coder", "name": "selected", "ignored": False}])
        self.assertEqual(lanes.ignored_inventory(listing), [{"binary": "unica-coder", "name": "outside_ignored", "ignored": True}])

    def test_unknown_executed_duplicate_and_required_skipped_raw_cases_fail(self):
        def replace_case(index, status):
            return lambda _plan, cases: cases.__setitem__(index, (cases[index][0], status))

        changes = {
            "unknown skipped": lambda _p, c: c.append(("unknown", "skipped")),
            "unknown passed": lambda _p, c: c.append(("unknown", "passed")),
            "unknown failed": lambda _p, c: c.append(("unknown", "failed")),
            "known ignored passed": replace_case(1, "passed"),
            "known ignored failed": replace_case(1, "failed"),
            "required skipped": replace_case(0, "skipped"),
            "duplicate ignored": lambda _p, c: c.append(c[1]),
            "duplicate assigned": lambda _p, c: c.append(c[0]),
            "skipped and failure": replace_case(1, "conflict"),
            "skipped after attempt": replace_case(1, "attempted_skip"),
        }
        for name, change in changes.items():
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.run_archive_with_ignored(change=change)

        for status in ("skipped", "passed", "failed"):
            def other_required(plan, cases):
                case = {"binary": "unica-coder", "name": "other_partition_required", "ignored": False}
                plan["inventory"].append(case)
                plan["shards"][1].append(case)
                cases.append((case["name"], status))
            with self.subTest(other_partition_status=status), self.assertRaises(ValueError):
                self.run_archive_with_ignored("ordinary", change=other_required)

    def test_union_validator_rechecks_catalogue_annotations_and_duplicates(self):
        plan = manifest()
        ignored = {"binary": "package-ordinary", "name": "ignored", "ignored": True}
        plan["inventory"].append(ignored)
        plan["shards"][1].append(ignored)
        plan["ignoredInventory"] = [ignored]
        good = result(plan, 1)
        annotation = {"id": lanes.case_id(ignored), "status": "skipped", "reason": "native run-ignored annotation"}
        good["frameworkAnnotations"] = [annotation]
        lanes.validate_result(plan, good, 1)
        for extra in (
            {**annotation, "id": "unknown::ignored"}, {**annotation, "status": "passed"},
            {**annotation, "status": "failed"}, good["records"][0],
        ):
            with self.subTest(annotation=extra), self.assertRaises(ValueError):
                lanes.validate_result(plan, {**good, "frameworkAnnotations": [extra]}, 1)
        with self.assertRaises(ValueError):
            lanes.validate_result(plan, {**good, "frameworkAnnotations": [annotation, annotation]}, 1)
        with self.assertRaises(ValueError):
            lanes.validate_result(plan, {**good, "records": good["records"] + [annotation]}, 1)
        for catalogue in (None, {}, [{}], [{**ignored, "ignored": False}], [ignored, ignored], []):
            malformed = {**plan, "ignoredInventory": catalogue}
            with self.subTest(catalogue=catalogue), self.assertRaises(ValueError):
                lanes.validate_result(malformed, good, 1)
        missing = {key: value for key, value in plan.items() if key != "ignoredInventory"}
        with self.assertRaises(ValueError):
            lanes.validate_result(missing, good, 1)
        changed = copy.deepcopy(plan)
        changed["inventory"][0]["ignored"] = True
        with self.assertRaises(ValueError):
            lanes.validate_result(changed, good, 1)

    def test_assigned_failure_and_retry_nodes_survive_filtering_with_honest_counts(self):
        for final in ("", '<failure message="assertion failed">final failure</failure>'):
            with self.subTest(final=final), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                plan = manifest("receipt")
                selected = plan["inventory"][0]
                ignored = {"binary": selected["binary"], "name": "outside", "ignored": True}
                plan["ignoredInventory"] = [ignored]
                raw = (f'<testsuites tests="2" skipped="1"><testsuite name="{selected["binary"]}" tests="2" skipped="1">'
                       f'<testcase name="{selected["name"]}" time="1"><rerunFailure message="assertion failed">original attempt</rerunFailure>'
                       + final + '</testcase><testcase name="outside"><skipped message="ignored"/></testcase></testsuite></testsuites>').encode()
                source = root / "native.xml"
                source.write_bytes(raw)
                owned, records, annotations = lanes.assigned_junit(source, root / "result", plan, plan["shards"][0])
                self.assertEqual((root / "result/junit.raw.xml").read_bytes(), raw)
                parsed = ET.parse(owned).getroot()
                self.assertEqual(parsed.get("tests"), "1")
                self.assertEqual(parsed.get("skipped"), "0")
                self.assertEqual(parsed.get("failures"), "1" if final else "0")
                self.assertEqual(parsed.find("testsuite/testcase/rerunFailure").text, "original attempt")
                self.assertEqual(records[0]["status"], "failed" if final else "passed")
                self.assertEqual(len(annotations), 1)
                with patch.object(lanes.runner.allure_results, "ignore_reasons", return_value={}):
                    count = lanes.runner.emit_rust(root / "result", "queue", "ubuntu-latest", owned)
                self.assertEqual(count, 2, "one original failed attempt and the assigned final result remain visible")


class ShadowArchiveTests(unittest.TestCase):
    def run_shadow(self, lane, packages, *, listing_failure=False, full_failure=False, ignored_annotation=False):
        """Run the real archive control flow with a bounded nextest CLI oracle."""
        with tempfile.TemporaryDirectory() as temporary, patch.dict(
            lanes.os.environ, {"GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "2"}
        ):
            root = Path(temporary)
            prepared = root / "prepared"
            prepared.mkdir()
            archive = prepared / "tests.tar.zst"
            archive.write_bytes(b"immutable test archive fixture")
            package = "ureq" if lane == "vendor" else "unica-coder"
            case = {"binary": package, "name": "lane_case", "ignored": False}
            plan = manifest(lane)
            plan.update(workspace=str(root.resolve()), nextestVersion="fixture nextest", runId="123", attempt="2",
                        inventory=[case], shards=[[case]],
                        archiveSha256=lanes.prepared_binary.digest(archive), archiveBytes=archive.stat().st_size)
            if ignored_annotation:
                plan["ignoredInventory"] = [{"binary": package, "name": "outside_lane_ignored", "ignored": True}]
            lanes.write(prepared / "lane.json", plan)
            decision = root / "decision.json"
            lanes.write(decision, {"mode": "affected", "mandatoryMode": "full", "affectedAdmission": False,
                                   "headSha": plan["identity"]["sha"], "packages": packages})
            commands = []

            def nextest(command, _root, *, capture=False, **_):
                commands.append(command)
                expressions = [command[i + 1] for i, arg in enumerate(command) if arg == "-E"]
                if command[2] == "list":
                    requested = {name for expression in expressions for name in re.findall(r"package\(=([^)]*)\)", expression)}
                    if listing_failure or requested - {package}:
                        return subprocess.CompletedProcess(command, 2, stdout="NoPackageMatch"), 0.1
                    cases = {case["name"]: {"ignored": False}}
                    # Nextest unions repeated -E: a package predicate would
                    # also select a real binary case outside the lane filter.
                    if len(expressions) > 1:
                        cases["unrelated_ordinary_case"] = {"ignored": False}
                    document = {"rust-suites": {package: {"binary-id": package, "testcases": cases}}}
                    return subprocess.CompletedProcess(command, 0, stdout=json.dumps(document)), 0.1
                junit = root / "target/nextest" / plan["nextestProfile"] / "junit.xml"
                junit.parent.mkdir(parents=True, exist_ok=True)
                annotation = '<testcase name="outside_lane_ignored"><skipped message="native ignored annotation"/></testcase>' if ignored_annotation else ''
                junit.write_text(f'<testsuites><testsuite name="{package}"><testcase name="lane_case"/>{annotation}</testsuite></testsuites>')
                return subprocess.CompletedProcess(command, 1 if full_failure else 0), 0.2

            destination = root / "result"
            with patch.object(lanes.prepared_binary, "identity", return_value=plan["identity"]), \
                 patch.object(lanes.prepared_binary, "output", return_value=plan["nextestVersion"]), \
                 patch.object(lanes, "signature"), patch.object(lanes.runner, "emit_rust"), \
                 patch.object(lanes, "timed", side_effect=nextest):
                status = lanes.run_archive(root, prepared, destination, lane, "queue", "ubuntu-latest", 1, None, decision)
            if ignored_annotation:
                self.assertEqual((destination / "junit.raw.xml").read_bytes(),
                                 (destination / "shadow/junit.raw.xml").read_bytes())
            return status, commands, json.loads((destination / "shadow.json").read_text()), json.loads((destination / "shard.json").read_text())

    def test_shadow_keeps_known_unassigned_ignored_annotations_out_of_both_outcome_sets(self):
        status, commands, shadow, full = self.run_shadow("receipt", ["unica-coder"], ignored_annotation=True)
        self.assertEqual(status, 0)
        self.assertEqual(len(commands), 3)
        self.assertEqual(shadow["records"], full["records"])
        self.assertEqual(shadow["frameworkAnnotations"], full["frameworkAnnotations"])
        self.assertEqual(len(shadow["records"]), 1)
        self.assertEqual(shadow["frameworkAnnotations"][0]["id"], "unica-coder::outside_lane_ignored")

    def test_vendor_candidate_outside_archive_records_no_shadow_after_full_pass(self):
        status, commands, shadow, full = self.run_shadow("vendor", ["unica-dcs", "unica-coder"])
        self.assertEqual(status, 0)
        self.assertEqual(len(commands), 1)
        self.assertEqual(commands[0][2], "run")
        self.assertEqual(full["records"], [{"id": "ureq::lane_case", "status": "passed"}])
        self.assertEqual(shadow["selected"], 0)
        self.assertTrue(shadow["shadowNotRun"])
        self.assertEqual(shadow["selectedPackages"], [])

    def test_nonempty_vendor_and_receipt_candidates_intersect_lane_once(self):
        for lane, package in (("vendor", "ureq"), ("receipt", "unica-coder")):
            with self.subTest(lane=lane):
                status, commands, shadow, full = self.run_shadow(lane, [package, "unica-dcs"])
                self.assertEqual(status, 0)
                self.assertEqual([command[2] for command in commands], ["run", "list", "run"])
                self.assertEqual(shadow["selectedPackages"], [package])
                self.assertEqual(shadow["selected"], 1)
                self.assertFalse(shadow["shadowNotRun"])
                self.assertEqual(len(full["records"]), 1)
                for command in commands[1:]:
                    self.assertEqual(command.count("-E"), 1)
                    expression = command[command.index("-E") + 1]
                    self.assertIn(lanes.filters(lane)[1], expression)
                    self.assertIn(" & ", expression)
                    self.assertNotIn("package(=unica-dcs)", expression)

    def test_known_shadow_errors_and_mandatory_failure_remain_fail_closed(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_shadow("vendor", ["ureq"], listing_failure=True)
        with self.assertRaisesRegex(ValueError, "required shard test failed"):
            self.run_shadow("vendor", ["unica-dcs"], full_failure=True)


class PreviousInventoryTests(unittest.TestCase):
    """The plan is red when a test target, job, result or selection vanished since the last verified run."""

    HOSTS = ["ubuntu-latest", "macos-14"]

    def jobs(self, profile="queue", drop=()):
        names = {f"{phase}-rust-{lane}": {"result": "success" if lane in lanes.lanes(profile) else "skipped"}
                 for lane in lanes.LANES for phase in ("build", "run")}
        names["shadow-component-selection"] = {"result": "success"}
        names["classify-changes"] = {"result": "success"}
        names["run-rust-ordinary-medium"] = {"result": "success" if profile == "pr" else "skipped"}
        return {name: value for name, value in names.items() if name not in drop}

    def collect(self, root, profile="queue", hosts=None, mutate=None):
        for host in hosts or self.HOSTS:
            for lane in lanes.lanes(profile):
                plan = manifest(lane, host, profile)
                if mutate:
                    mutate(plan)
                lanes.write(root / host / lane / "lane.json", plan)
                for index in range(1, len(plan["shards"]) + 1):
                    lanes.write(root / host / lane / str(index) / "shard.json", result(plan, index))
                if profile == "pr" and (host, lane) == ("ubuntu-latest", "ordinary"):
                    medium = {**result({**plan, "shards": [plan["inventory"]]}, 1), "nextestProfile": "queue",
                              "reason": "shadow selection is full; all medium tests", "expression": None, "packages": []}
                    lanes.write(root / host / "medium" / "medium.json", medium)

    def previous(self, profile="queue", *, reset=False, removed=(), jobs=None, mutate=None, hosts=None):
        with tempfile.TemporaryDirectory() as temporary, \
             patch.dict(lanes.os.environ, {"CI_LANE_JOB_RESULTS": json.dumps(jobs or self.jobs(profile))}, clear=True):
            root = Path(temporary)
            self.collect(root, profile, hosts, mutate)
            lanes.verify_union(root, profile, hosts or self.HOSTS, "a" * 40, self.baseline(None, reset=True))
            inventory = json.loads((root / "inventory.json").read_text())
        return self.baseline(inventory, reset=reset, removed=removed)

    def baseline(self, inventory, *, reset=False, removed=()):
        return {"schemaVersion": 1, "source": None if inventory is None else {"headSha": "b" * 40},
                "inventory": inventory, "explanations": {"range": "b..HEAD", "reset": reset, "removedTargets": list(removed)}}

    def verify(self, baseline, profile="queue", *, jobs=None, mutate=None, hosts=None):
        with tempfile.TemporaryDirectory() as temporary, \
             patch.dict(lanes.os.environ, {"CI_LANE_JOB_RESULTS": json.dumps(jobs or self.jobs(profile))}, clear=True):
            root = Path(temporary)
            self.collect(root, profile, hosts, mutate)
            try:
                return lanes.verify_union(root, profile, hosts or self.HOSTS, "a" * 40, baseline)
            except ValueError as error:
                report = root / "verification.json"
                self.report = json.loads(report.read_text()) if report.is_file() else None
                raise error

    @staticmethod
    def without_target(binary):
        def mutate(plan):
            plan["binaries"] = [item for item in plan["binaries"] if item != binary]
        return mutate

    def test_unchanged_inventory_passes_and_records_the_source(self):
        report = self.verify(self.previous())
        self.assertTrue(report["mandatoryComplete"])
        self.assertEqual(report["previousInventory"]["source"], {"headSha": "b" * 40})

    def test_vanished_test_target_fails_unless_a_commit_declares_its_removal(self):
        with self.assertRaisesRegex(ValueError, "test target unica-coder disappeared"):
            self.verify(self.previous(), mutate=self.without_target("unica-coder"))
        self.assertFalse(self.report["mandatoryComplete"])
        report = self.verify(self.previous(removed=["unica-coder"]), mutate=self.without_target("unica-coder"))
        self.assertEqual(report["previousInventory"]["removedTargets"], ["unica-coder"])

    def test_target_loss_is_found_across_profiles_but_selection_only_within_one(self):
        # A pull request runs ordinary small tests only; its archive still has every target.
        with self.assertRaisesRegex(ValueError, "ubuntu-latest/ordinary: test target ureq disappeared"):
            self.verify(self.previous("queue"), "pr", jobs=self.jobs("pr"), hosts=["ubuntu-latest"],
                        mutate=self.without_target("ureq"))
        self.assertTrue(self.verify(self.previous("queue"), "pr", jobs=self.jobs("pr"), hosts=["ubuntu-latest"])["mandatoryComplete"])

    def test_vanished_job_fails(self):
        with self.assertRaisesRegex(ValueError, "job shadow-component-selection disappeared"):
            # The in-run check names the missing shadow too; the comparison names the vanished job.
            self.verify(self.previous(), jobs=self.jobs(drop=("shadow-component-selection",)))
        with self.assertRaisesRegex(ValueError, "job classify-changes disappeared"):
            self.verify(self.previous(), jobs=self.jobs(drop=("classify-changes",)))

    def test_vanished_configuration_and_its_results_fail_within_one_profile(self):
        previous = self.previous()
        with self.assertRaisesRegex(ValueError, "macos-14/receipt: configuration and its results disappeared"):
            with patch.object(lanes, "lanes", side_effect=lambda profile: ("ordinary",) if profile == "queue" else lanes.LANES):
                self.verify(previous, jobs={**self.jobs(), **{f"{phase}-rust-{lane}": {"result": "skipped"}
                                                   for lane in ("receipt", "vendor") for phase in ("build", "run")}})

    def test_unexpectedly_empty_selection_fails(self):
        def empty(plan):
            # The target is still archived, but the profile filter now selects nothing from it.
            kept = [case for case in plan["inventory"] if case["binary"] != "package-receipt"]
            plan["inventory"] = kept + [{"binary": "unica-coder", "name": "other", "ignored": False}]
            plan["shards"] = [plan["inventory"]]
        with self.assertRaisesRegex(ValueError, "selection of test target package-receipt became empty"):
            self.verify(self.previous(), mutate=lambda plan: empty(plan) if plan["lane"] == "receipt" else None)
        with self.assertRaisesRegex(ValueError, "nonempty complete plan"):
            self.verify(self.previous(), mutate=lambda plan: plan.update(inventory=[], shards=[[]] * len(plan["shards"])))

    def test_removed_cases_are_reported_without_failing(self):
        def rename(plan):
            for part in plan["shards"]:
                for case in part:
                    case["name"] += "_renamed"
            plan["inventory"] = sum(plan["shards"], [])
        report = self.verify(self.previous(), mutate=rename)
        self.assertIn("ubuntu-latest/ordinary", report["previousInventory"]["removedCases"])

    def test_missing_baseline_requires_an_explicit_reset_and_ci_requires_the_comparison(self):
        with self.assertRaisesRegex(ValueError, "Rust-inventory-baseline: reset"):
            self.verify(self.baseline(None))
        self.assertTrue(self.verify(self.baseline(None, reset=True))["mandatoryComplete"])
        # Release lines and manual runs may outlive the artifact retention.
        self.assertIsNone(self.verify(self.baseline(None), "main", jobs=self.jobs("main"))["previousInventory"]["source"])
        report = self.verify(self.previous(reset=True), mutate=self.without_target("unica-coder"))
        self.assertTrue(report["previousInventory"]["resetSuppressed"])
        with self.assertRaisesRegex(ValueError, "not compared with the previous inventory"):
            self.verify(None)
        with self.assertRaises(ValueError):
            self.verify({**self.baseline(None), "explanations": {"reset": "yes", "removedTargets": []}})

    def test_equivalent_profiles_compare_selection_and_configurations(self):
        self.assertTrue(lanes.same_selection("queue", "main"))
        self.assertFalse(lanes.same_selection("queue", "pr"))
        self.assertFalse(lanes.same_selection("queue", "large"))
        def drop_receipt_case(plan):
            if plan["lane"] == "receipt":
                plan["inventory"] = [{"binary": "unica-coder", "name": "other", "ignored": False}]
                plan["shards"] = [plan["inventory"]]
        with self.assertRaisesRegex(ValueError, "selection of test target package-receipt became empty"):
            self.verify(self.previous("queue"), "main", jobs=self.jobs("main"), mutate=drop_receipt_case)

    def test_windows_losses_in_large_stay_advisory(self):
        hosts = ["ubuntu-latest", "macos-14", "windows-latest"]
        previous = self.previous("large", jobs=self.jobs("large"), hosts=hosts)
        report = self.verify(previous, "large", jobs=self.jobs("large"), hosts=hosts,
                             mutate=lambda plan: self.without_target("ureq")(plan) if plan["runner"] == "windows-latest" else None)
        self.assertTrue(report["mandatoryComplete"])
        self.assertFalse(report["fullMatrixPassed"])
        self.assertTrue(all("windows-latest" in item for item in report["advisoryFailures"]))


class PullRequestMediumTests(unittest.TestCase):
    """A pull request also runs medium tests of affected packages from its own ordinary archive."""

    SHA = "a" * 40

    def decision(self, packages, *, mode="affected", sha=None):
        return {"mode": mode, "mandatoryMode": "full", "affectedAdmission": False,
                "headSha": sha or self.SHA, "packages": packages}

    def test_selection_narrows_to_archived_affected_packages_and_falls_back_to_all(self):
        binaries = ["unica-coder", "unica-coder::cli_contract", "unica-workspace"]
        self.assertEqual(lanes.medium_selection(self.decision(["unica-workspace", "unica-docs"]), binaries, self.SHA),
                         ("package(=unica-workspace)", ["unica-workspace"], "affected packages and their reverse consumers"))
        self.assertEqual(lanes.medium_selection(self.decision(["unica-docs"]), binaries, self.SHA)[:2], ("", []))
        for decision, reason in ((None, "unavailable"), (self.decision(["unica-coder"], sha="b" * 40), "another revision"),
                                 (self.decision([], mode="full"), "full"), ({**self.decision(["x"]), "affectedAdmission": True}, "invalid")):
            with self.subTest(reason=reason):
                expression, packages, why = lanes.medium_selection(decision, binaries, self.SHA)
                self.assertIsNone(expression)
                self.assertEqual(packages, [])
                self.assertIn(reason, why)
                self.assertIn("all medium tests", why)

    def run_medium(self, decision, *, failure=False, missing_junit=False, cases=("medium_case",), host="ubuntu-latest"):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(lanes.os.environ, {"GITHUB_RUN_ID": "1", "GITHUB_RUN_ATTEMPT": "1"}):
            root = Path(temporary).resolve()
            prepared = root / "prepared"
            prepared.mkdir()
            archive = prepared / "tests.tar.zst"
            archive.write_bytes(b"pr ordinary archive")
            plan = manifest("ordinary", profile="pr")
            plan.update(workspace=str(root), nextestVersion="fixture nextest", runId="1", attempt="1",
                        archiveSha256=lanes.prepared_binary.digest(archive), archiveBytes=archive.stat().st_size)
            lanes.write(prepared / "lane.json", plan)
            shadow = root / "decision.json"
            if decision is not None:
                lanes.write(shadow, decision)
            commands = []

            # Without -E the queue profile also lists the small cases the PR planned.
            small = [case["name"] for case in plan["inventory"]]

            def suites(command):
                found = {"unica-coder": list(cases)}
                if "-E" not in command and cases:
                    found["package-ordinary"] = small
                return found

            def nextest(command, _root, *, capture=False, **_):
                commands.append(command)
                if command[2] == "list":
                    document = {"rust-suites": {binary: {"binary-id": binary, "testcases": {name: {"ignored": False} for name in names}}
                                                for binary, names in suites(command).items()}}
                    return subprocess.CompletedProcess(command, 0, stdout=json.dumps(document)), 0.1
                if not missing_junit:
                    junit = root / "target/nextest/queue/junit.xml"
                    junit.parent.mkdir(parents=True, exist_ok=True)
                    junit.write_text("<testsuites>" + "".join(
                        f'<testsuite name="{binary}">' + "".join(f'<testcase name="{name}"/>' for name in names) + "</testsuite>"
                        for binary, names in suites(command).items()) + "</testsuites>")
                return subprocess.CompletedProcess(command, 1 if failure else 0), 0.2

            destination = root / "medium"
            with patch.object(lanes.prepared_binary, "identity", return_value=plan["identity"]), \
                 patch.object(lanes.prepared_binary, "output", return_value="fixture nextest"), \
                 patch.object(lanes, "fixture_binary_environment", return_value={}), \
                 patch.object(lanes, "timed", side_effect=nextest):
                try:
                    status = lanes.run_medium(root, prepared, destination, host, shadow)
                finally:
                    written = destination / "medium.json"
                    self.medium = json.loads(written.read_text()) if written.is_file() else None
            return status, commands

    def test_affected_medium_runs_from_the_same_archive_with_the_queue_profile(self):
        status, commands = self.run_medium(self.decision(["unica-coder", "unica-docs"]))
        self.assertEqual(status, 0)
        self.assertEqual([command[2] for command in commands], ["list", "run"])
        for command in commands:
            self.assertEqual(command[command.index("--profile") + 1], "queue")
            self.assertEqual(command[command.index("-E") + 1], "package(=unica-coder)")
            self.assertNotIn("build", command)
        self.assertEqual([record["status"] for record in self.medium["records"]], ["passed"])

    def test_unknown_or_full_selection_runs_every_medium_test(self):
        for decision in (None, self.decision([], mode="full"), self.decision(["unica-coder"], sha="b" * 40)):
            with self.subTest(decision=decision):
                _, commands = self.run_medium(decision)
                self.assertEqual([command[2] for command in commands], ["list", "run"])
                self.assertTrue(all("-E" not in command for command in commands))
                self.assertIn("all medium tests", self.medium["reason"])

    def test_failed_missing_or_unexpectedly_empty_medium_results_fail(self):
        for decision, options in ((self.decision(["unica-coder"]), {"failure": True}),
                                  (self.decision(["unica-coder"]), {"missing_junit": True}),
                                  (None, {"cases": ()})):
            with self.subTest(options=options), self.assertRaises(ValueError):
                self.run_medium(decision, **options)
            self.assertIsNotNone(self.medium)
        # Narrowed packages may hold no tests outside large: listed, empty, explained.
        status, commands = self.run_medium(self.decision(["unica-coder"]), cases=())
        self.assertEqual((status, [command[2] for command in commands], self.medium["reason"]), (0, ["list"], lanes.MEDIUM_NO_TESTS))
        status, commands = self.run_medium(self.decision(["unica-docs"]))
        self.assertEqual((status, commands, self.medium["plan"], self.medium["reason"]), (0, [], [], lanes.MEDIUM_NO_TESTS))
        with self.assertRaisesRegex(ValueError, "Linux only"):
            self.run_medium(self.decision(["unica-coder"]), host="macos-14")

    def test_recorded_expression_must_match_its_packages(self):
        plan = manifest("ordinary", profile="pr")
        medium = {**result(plan, 1), "nextestProfile": "queue", "reason": "x", "expression": "package(=other)",
                  "packages": ["package-ordinary"]}
        with self.assertRaisesRegex(ValueError, "differs from its selected packages"):
            lanes.validate_medium(plan, medium)

    def test_verifier_requires_the_medium_result_of_a_pull_request(self):
        plan = manifest("ordinary", profile="pr")
        medium_plan = plan["inventory"] + [{"binary": "package-ordinary", "name": "medium_case", "ignored": False}]
        medium = {**result({**plan, "shards": [medium_plan]}, 1), "nextestProfile": "queue", "reason": "x",
                  "expression": None, "packages": []}
        jobs = {f"{phase}-rust-{lane}": {"result": "success" if lane == "ordinary" else "skipped"}
                for lane in lanes.LANES for phase in ("build", "run")}
        jobs.update({"shadow-component-selection": {"result": "success"}, "run-rust-ordinary-medium": {"result": "success"}})
        cases = {"complete": (medium, jobs, None), "missing": (None, jobs, "missing or duplicate medium result"),
                 "failed": ({**medium, "exitCode": 1}, jobs, "required shard test failed"),
                 "skipped job": (medium, {**jobs, "run-rust-ordinary-medium": {"result": "skipped"}}, "did not finish as success"),
                 "empty": ({**medium, "plan": [], "records": []}, jobs, "unexpectedly empty"),
                 "truncated": ({**medium, "plan": medium_plan[1:], "records": medium["records"][1:]}, jobs, "omits 1 planned case"),
                 "foreign expression": ({**medium, "expression": "package(=other)"}, jobs, "differs from"),
                 "narrowed": ({**medium, "expression": "package(=package-ordinary)", "packages": ["package-ordinary"]}, jobs, None),
                 "small shard failed": ({**medium, "exitCode": 1}, jobs, r"ubuntu-latest/ordinary medium: a required shard test failed"),
                 "selection differs": ({**medium, "expression": "", "packages": [], "reason": lanes.MEDIUM_NO_TESTS, "plan": [], "records": []},
                                       jobs, "differs from the shadow decision")}
        for name, (document, needs, error) in cases.items():
            with self.subTest(name), tempfile.TemporaryDirectory() as temporary, \
                 patch.dict(lanes.os.environ, {"CI_LANE_JOB_RESULTS": json.dumps(needs)}, clear=True):
                root = Path(temporary)
                lanes.write(root / "ordinary/lane.json", plan)
                lanes.write(root / "ordinary/1/shard.json", {**result(plan, 1), "exitCode": 1} if name == "small shard failed" else result(plan, 1))
                if document is not None:
                    lanes.write(root / "medium/medium.json", document)
                if name in ("narrowed", "selection differs"):
                    lanes.write(root / "shadow/selection.json", {"mode": "affected", "mandatoryMode": "full", "affectedAdmission": False,
                                                                  "headSha": "a" * 40, "packages": ["package-ordinary"]})
                baseline = {"inventory": None, "explanations": {"reset": True, "removedTargets": []}}
                if error is None:
                    report = lanes.verify_union(root, "pr", ["ubuntu-latest"], "a" * 40, baseline)
                    self.assertEqual(report["medium"][0]["tests"], len(document["plan"]))
                else:
                    with self.assertRaisesRegex(ValueError, error):
                        lanes.verify_union(root, "pr", ["ubuntu-latest"], "a" * 40, baseline)
        with tempfile.TemporaryDirectory() as temporary, patch.dict(lanes.os.environ, {}, clear=True):
            lanes.write(Path(temporary) / "medium.json", medium)
            with self.assertRaisesRegex(ValueError, "outside a pull request"):
                lanes.verify_union(Path(temporary), "queue", ["ubuntu-latest"], "a" * 40)


class HostCapabilitySkipTests(unittest.TestCase):
    """A missing host capability is a visible skip with its reason, refused where required (#1893)."""

    RUST = Path(__file__).resolve().parents[2] / "crates/unica-process/src/platform/host_capability.rs"

    def test_capability_names_match_the_rust_helper_and_the_runner_table(self):
        source = self.RUST.read_text(encoding="utf-8")
        self.assertEqual(set(re.findall(r'=>\s*\{?\s*"([a-z0-9-]+)"', source)), set(lanes.HOST_CAPABILITIES))
        self.assertIn(f'"{lanes.HOST_CAPABILITY_SKIPS_ENV}"', source)
        self.assertEqual(set(lanes.REQUIRED_HOST_CAPABILITIES), {"ubuntu-latest", "macos-14", "windows-latest"})
        for runner, required in lanes.REQUIRED_HOST_CAPABILITIES.items():
            with self.subTest(runner=runner):
                self.assertLessEqual(required, lanes.HOST_CAPABILITIES)

    def run_with_skip(self, capability, *, host_status="", missing=False, runner="ubuntu-latest"):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(lanes.os.environ, {}, clear=True):
            root = Path(temporary).resolve()
            prepared = root / "prepared"
            prepared.mkdir()
            archive = prepared / "tests.tar.zst"
            archive.write_bytes(b"capability archive")
            plan = manifest("receipt", host=runner, profile="queue")
            plan.update(workspace=str(root), nextestVersion="fixture nextest",
                        archiveSha256=lanes.prepared_binary.digest(archive), archiveBytes=archive.stat().st_size)
            lanes.write(prepared / "lane.json", plan)
            case = plan["inventory"][0]

            def nextest(command, _root, *, env=None, **_):
                skips = Path(env[lanes.HOST_CAPABILITY_SKIPS_ENV])
                name = "absent_case" if missing else case["name"]
                (skips / "record.json").write_text(json.dumps({
                    "binary": case["binary"], "name": name, "attempt": "1",
                    "capability": capability, "reason": "creating a symlink needs a privilege"}))
                junit = root / "target/nextest" / plan["nextestProfile"] / "junit.xml"
                junit.parent.mkdir(parents=True, exist_ok=True)
                junit.write_text(f'<testsuites><testsuite name="{case["binary"]}"><testcase name="{case["name"]}">'
                                 f'{host_status}</testcase></testsuite></testsuites>')
                return subprocess.CompletedProcess(command, 1 if host_status else 0), 0.1

            destination = root / "result"
            try:
                with patch.object(lanes.prepared_binary, "identity", return_value=plan["identity"]), \
                     patch.object(lanes.prepared_binary, "output", return_value="fixture nextest"), \
                     patch.object(lanes, "signature"), patch.object(lanes.runner, "emit_rust"), \
                     patch.object(lanes, "timed", side_effect=nextest):
                    lanes.run_archive(root, prepared, destination, "receipt", "queue", runner, 1, None)
            finally:
                written = destination / "shard.json"
                self.shard = json.loads(written.read_text()) if written.is_file() else None
                assigned = destination / "junit.assigned.xml"
                self.junit = assigned.read_text() if assigned.is_file() else ""

    def test_a_capability_the_runner_lacks_turns_pass_into_a_visible_skip(self):
        self.run_with_skip("symlink", runner="windows-latest")
        self.assertEqual(self.shard["records"][0]["status"], "skipped")
        self.assertEqual(self.shard["records"][0]["reason"], "host capability symlink: creating a symlink needs a privilege")
        self.assertIn('<skipped message="host capability symlink', self.junit)

    def test_a_capability_required_on_the_runner_cannot_be_skipped(self):
        with self.assertRaisesRegex(ValueError, "host capability symlink is required on ubuntu-latest"):
            self.run_with_skip("symlink")
        self.run_with_skip("case-insensitive-filesystem")
        self.assertEqual(self.shard["records"][0]["status"], "skipped")

    def test_unknown_capability_stray_record_and_failed_test_are_not_skips(self):
        with self.assertRaisesRegex(ValueError, "malformed host capability skip record"):
            self.run_with_skip("teleportation", runner="windows-latest")
        with self.assertRaisesRegex(ValueError, "case missing from JUnit"):
            self.run_with_skip("symlink", runner="windows-latest", missing=True)
        with self.assertRaisesRegex(ValueError, "required shard test failed"):
            self.run_with_skip("symlink", runner="windows-latest", host_status='<failure message="boom"/>')
        self.assertEqual(self.shard["records"][0]["status"], "failed")

    def test_only_the_final_attempt_decides_a_retried_skip(self):
        tree = ET.ElementTree(ET.fromstring(
            '<testsuites><testsuite name="p"><testcase name="t"><flakyFailure message="x"/></testcase>'
            '<testcase name="u"><flakyFailure message="x"/></testcase></testsuite></testsuites>'))
        lanes.apply_capability_skips(tree, {"p::t": (1, "host capability symlink: a"), "p::u": (2, "host capability symlink: b")},
                                     {"p::t", "p::u"})
        cases = {case.get("name"): case.find("skipped") for case in tree.getroot().iter("testcase")}
        self.assertIsNone(cases["t"], "the passing retry did not skip")
        self.assertEqual(cases["u"].get("message"), "host capability symlink: b")
        # The whole path keeps the retried capability skip: no conflict, one skipped record.
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "junit.xml"
            source.write_text('<testsuites><testsuite name="p"><testcase name="u"><flakyFailure message="x"/></testcase>'
                              '</testsuite></testsuites>')
            plan = manifest("receipt")
            plan["ignoredInventory"] = []
            _, records, _ = lanes.assigned_junit(source, Path(temporary) / "out", plan,
                                                 [{"binary": "p", "name": "u", "ignored": False}],
                                                 {"p::u": (2, "host capability symlink: b")})
            self.assertEqual(records, [{"id": "p::u", "status": "skipped", "reason": "host capability symlink: b"}])
            entries = lanes.allure_results.junit_records(Path(temporary) / "out/junit.assigned.xml", runner="windows-latest",
                                                         profile="queue", reasons={})
            self.assertEqual([entry["status"] for entry in entries if entry["name"] == "u"], ["broken", "skipped"])

    def test_only_a_capability_reason_excuses_a_required_case(self):
        record = {"id": "package::case", "status": "skipped"}
        self.assertEqual(lanes.capability_skip_refusal({**record, "reason": "flaky"}, "windows-latest"),
                         "a required shard test did not execute")
        self.assertIsNone(lanes.capability_skip_refusal({**record, "reason": "host capability symlink: x"}, "windows-latest"))
        self.assertIn("required on macos-14", lanes.capability_skip_refusal(
            {**record, "reason": "host capability case-insensitive-filesystem: x"}, "macos-14"))


if __name__ == "__main__":
    unittest.main()
