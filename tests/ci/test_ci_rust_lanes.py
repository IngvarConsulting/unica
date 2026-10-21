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
            "archiveSha256": "archive-" + lane, "inventory": sum(parts, []), "ignoredInventory": [], "shards": parts}
    if lane == "ordinary":
        document["fixtureBinary"] = {"path": "x86_64-unknown-linux-gnu/debug/unica", "sha256": "0" * 64, "bytes": 1}
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

                def nextest(command, _root, *, capture=False):
                    calls.append(command[2])
                    if command[2] == "archive":
                        (root / "prepared/tests.tar.zst").write_bytes(b"same archive")
                    return subprocess.CompletedProcess(command, 0, stdout=json.dumps(listed)), 0.01

                with patch.object(lanes.prepared_binary, "identity", return_value=identity), \
                     patch.object(lanes.prepared_binary, "output", return_value="fixture nextest"), \
                     patch.object(lanes, "signature"), patch.object(lanes, "timed", side_effect=nextest):
                    lanes.build(root, root / "prepared", lane, "pr", "ubuntu-latest", "main")
                plan = json.loads((root / "prepared/lane.json").read_text())
                self.assertEqual(calls, ["archive", "list", "list"])
                if lane == "ordinary":
                    self.assertEqual(plan["fixtureBinary"], {"path": "x86_64-unknown-linux-gnu/debug/unica",
                        "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "bytes": binary.stat().st_size})
                else:
                    self.assertNotIn("fixtureBinary", plan)

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

    def run_fixture(self, lane="ordinary", mutate=None, *, shadow=False):
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
                calls.append((command[2], kwargs["env"]))
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
                    self.assertIsNone(kwargs["env"])
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
            return calls

    def test_actual_timed_full_and_shadow_use_the_same_binding_while_other_lanes_are_unchanged(self):
        calls = self.run_fixture(shadow=True)
        self.assertEqual([kind for kind, _ in calls], ["run", "list", "run"])
        self.assertEqual(calls[0][1], calls[2][1])
        self.assertIsNone(calls[1][1], "listing does not receive fixture execution inputs")
        for lane in ("receipt", "vendor"):
            self.assertEqual(self.run_fixture(lane), [("run", None)])

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
        self.assertEqual(len(allure), 1)

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

            def nextest(command, _root, *, capture=False):
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


if __name__ == "__main__":
    unittest.main()
