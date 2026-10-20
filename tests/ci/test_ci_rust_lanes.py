"""Lane/shard inventory is a fail-closed partition of the original full suite."""
from __future__ import annotations
import copy
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import ci_rust_lanes as lanes


def manifest(lane="ordinary", host="ubuntu-latest", profile="queue"):
    parts = [[{"binary": f"package-{lane}", "name": f"case_{i}", "ignored": False}] for i in range(lanes.shards(profile, lane))]
    return {"schemaVersion": 1, "identity": {"sha": "a" * 40}, "profile": profile, "runner": host,
            "lane": lane, "selection": list(lanes.selection(lane)), "nextestProfile": lanes.runner.nextest_profile(profile, lanes.selection(lane)),
            "archiveSha256": "archive-" + lane, "inventory": sum(parts, []), "shards": parts}


def result(plan, index):
    return {"schemaVersion": 1, "sha": plan["identity"]["sha"], "profile": plan["profile"], "runner": plan["runner"],
            "lane": plan["lane"], "shard": index, "archiveSha256": plan["archiveSha256"],
            "plan": plan["shards"][index - 1], "exitCode": 0,
            "records": [{"id": lanes.case_id(case), "status": "passed"} for case in plan["shards"][index - 1]]}


class RustLaneTests(unittest.TestCase):
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


class ShadowArchiveTests(unittest.TestCase):
    def run_shadow(self, lane, packages, *, listing_failure=False, full_failure=False):
        """Run the real archive control flow with a bounded nextest CLI oracle."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            prepared = root / "prepared"
            prepared.mkdir()
            archive = prepared / "tests.tar.zst"
            archive.write_bytes(b"immutable test archive fixture")
            package = "ureq" if lane == "vendor" else "unica-coder"
            case = {"binary": package, "name": "lane_case", "ignored": False}
            plan = manifest(lane)
            plan.update(workspace=str(root.resolve()), nextestVersion="fixture nextest",
                        inventory=[case], shards=[[case]],
                        archiveSha256=lanes.prepared_binary.digest(archive), archiveBytes=archive.stat().st_size)
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
                junit.write_text(f'<testsuites><testsuite name="{package}"><testcase name="lane_case"/></testsuite></testsuites>')
                return subprocess.CompletedProcess(command, 1 if full_failure else 0), 0.2

            destination = root / "result"
            with patch.object(lanes.prepared_binary, "identity", return_value=plan["identity"]), \
                 patch.object(lanes.prepared_binary, "output", return_value=plan["nextestVersion"]), \
                 patch.object(lanes, "signature"), patch.object(lanes.runner, "emit_rust"), \
                 patch.object(lanes, "timed", side_effect=nextest):
                status = lanes.run_archive(root, prepared, destination, lane, "queue", "ubuntu-latest", 1, None, decision)
            return status, commands, json.loads((destination / "shadow.json").read_text()), json.loads((destination / "shard.json").read_text())

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
