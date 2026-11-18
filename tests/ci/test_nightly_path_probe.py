"""Ночной путь в очереди: кого он касается, что строит и как сверяет учёт."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import yaml

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "ci"))
SPEC = importlib.util.spec_from_file_location("nightly_path_probe", ROOT / "scripts" / "ci" / "nightly-path-probe.py")
assert SPEC is not None and SPEC.loader is not None
probe = importlib.util.module_from_spec(SPEC)
sys.modules["nightly_path_probe"] = probe
SPEC.loader.exec_module(probe)
lanes = probe.ci_rust_lanes

SHA = "a" * 40


def manifest(lane: str, host: str, cases: tuple[str, ...] = ("first", "second")) -> dict:
    """Манифест полосы ночи в форме, которую пишет `ci_rust_lanes.build`."""
    binary = f"package-{lane}"
    inventory = [{"binary": binary, "name": name, "ignored": False} for name in cases]
    return {"schemaVersion": 1, "identity": {"sha": SHA, "os": "Linux", "target": "x86_64-unknown-linux-gnu"},
            "profile": probe.PROFILE, "runner": host, "lane": lane, "selection": list(lanes.selection(lane)),
            "nextestProfile": lanes.nextest_profile(probe.PROFILE, lane, host), "inventory": inventory,
            "shards": [inventory], "ignoredInventory": [], "binaries": [binary]}


def listing(lane: str = "ordinary", cases: tuple[str, ...] = ("first", "second")) -> dict:
    """Вывод `nextest list --message-format json` для одного бинаря."""
    binary = f"package-{lane}"
    return {"rust-suites": {binary: {"binary-id": binary, "testcases": {
        name: {"ignored": False, "filter-match": {"status": "matches"}} for name in cases}}}}


def listed(cases: tuple[str, ...] = ("first", "second")) -> dict:
    return probe.listed_configuration(listing(cases=cases), manifest(probe.LANE, probe.HOST)["identity"])


def night(**changes) -> dict:
    """База прошлой ночи: все раннеры, все полосы и список джоб."""
    manifests = [manifest(lane, host) for host in lanes.profile_hosts(probe.PROFILE)
                 for lane in lanes.lanes(probe.PROFILE, host)]
    inventory = lanes.inventory_summary(manifests, profile=probe.PROFILE, sha="b" * 40,
                                        jobs=["Rust (macos-15)", "Rust (windows-latest)", "Unica CI"])
    explanations = {"range": "b..HEAD", "reset": False, "removedTargets": [], "removedTests": [], "declaredBy": {}}
    explanations.update(changes)
    return {"schemaVersion": 1, "source": {"headSha": "b" * 40}, "inventory": inventory, "explanations": explanations}


class ScopeTests(unittest.TestCase):
    def test_only_changes_the_queue_does_not_execute_reach_the_nightly_path(self) -> None:
        probed = ["scripts/ci/ci_rust_lanes.py", ".github/workflows/release.yml", ".config/nextest.toml"]
        others = ["crates/unica-coder/src/lib.rs", "scripts/dev/x.py", "tests/ci/test_x.py", ".config/other.toml",
                  "docs/scripts/ci/x.md"]
        self.assertEqual(probe.touched(probed + others), sorted(probed))

    def test_a_queue_entry_without_ci_changes_builds_nothing(self) -> None:
        with mock.patch.object(probe, "changed_paths", return_value=["crates/a/src/lib.rs"]), \
             mock.patch.object(probe, "probe") as walked:
            self.assertEqual(probe.main(["--base", "b", "--head", "h"]), 0)
        walked.assert_not_called()

    def test_an_unread_diff_walks_the_path(self) -> None:
        for paths in (None, [".github/workflows/ci.yml"]):
            with self.subTest(paths=paths), mock.patch.object(probe, "changed_paths", return_value=paths), \
                 mock.patch.object(probe, "probe", return_value=["сломано"]) as walked:
                self.assertEqual(probe.main(["--base", "b", "--head", "h"]), 1)
                walked.assert_called_once()

    def test_the_probed_lane_is_one_the_night_builds_on_linux(self) -> None:
        planned = probe.plan()
        self.assertIn(probe.LANE, planned[probe.HOST])
        self.assertEqual(set(planned), set(lanes.HOSTS))
        with self.assertRaisesRegex(ValueError, "нет полосы"):
            probe.plan(lane="absent")


class AccountingTests(unittest.TestCase):
    def test_checked_lanes_match_their_configurations_and_others_are_not_missed(self) -> None:
        self.assertEqual(probe.account(manifest(probe.LANE, probe.HOST), night()), [])
        self.assertEqual(probe.account(manifest(probe.LANE, probe.HOST), night(), listed()), [])

    def test_a_case_lost_from_the_listed_ordinary_lane_is_reported(self) -> None:
        problems = probe.account(manifest(probe.LANE, probe.HOST), night(), listed(("first",)))
        self.assertEqual(len(problems), 1)
        self.assertIn(f"{probe.HOST}/{probe.LISTED}", problems[0])
        self.assertIn("package-ordinary::second", problems[0])

    def test_the_ordinary_listing_uses_the_nights_selection_and_profile(self) -> None:
        command = probe.listing_command()
        self.assertEqual(command[:3], ["cargo", "nextest", "list"])
        for argument in lanes.selection(probe.LISTED):
            self.assertIn(argument, command)
        self.assertEqual(command[command.index("--profile") + 1], lanes.nextest_profile(probe.PROFILE, probe.LISTED, probe.HOST))
        self.assertEqual(command[command.index("--run-ignored") + 1], "all")

    def test_a_lost_case_is_reported_unless_declared(self) -> None:
        current = manifest(probe.LANE, probe.HOST, ("first",))
        problems = probe.account(current, night())
        self.assertEqual(len(problems), 1)
        self.assertIn(f"{probe.HOST}/{probe.LANE}", problems[0])
        self.assertIn(f"package-{probe.LANE}::second", problems[0])
        self.assertEqual(probe.account(current, night(removedTests=[f"package-{probe.LANE}::second"])), [])

    def test_a_missing_night_inventory_is_a_refusal(self) -> None:
        baseline = night()
        baseline["inventory"] = None
        self.assertIn("no previous inventory", probe.account(manifest(probe.LANE, probe.HOST), baseline)[0])

    def test_a_manifest_of_another_selection_is_refused(self) -> None:
        current = manifest(probe.LANE, probe.HOST)
        current["nextestProfile"] = "default"
        with self.assertRaisesRegex(ValueError, "unexpected feature or test selection"):
            probe.account(current, night())


class WalkTests(unittest.TestCase):
    def walk(self, *, build: int = 0, lookup: int = 0, listed: int = 0,
             baseline: dict | None = None) -> tuple[list[str], list[list[str]]]:
        calls: list[list[str]] = []

        def execute(command: list[str]) -> int:
            calls.append(command)
            if "--phase" in command:
                if build == 0:
                    target = Path(command[command.index("--prepared") + 1]) / probe.LANE / "lane.json"
                    lanes.write(target, manifest(probe.LANE, probe.HOST))
                return build
            if lookup == 0:
                lanes.write(Path(command[command.index("--out") + 1]), night() if baseline is None else baseline)
            return lookup

        def read(command: list[str]) -> tuple[int, str]:
            calls.append(command)
            return listed, json.dumps(listing())

        with tempfile.TemporaryDirectory() as temporary:
            return probe.probe(Path(temporary) / "work", execute, read), calls

    def test_the_night_builder_builds_one_lane_then_accounting_runs(self) -> None:
        problems, calls = self.walk()
        self.assertEqual(problems, [])
        build, lookup, listing_call = calls
        self.assertEqual(listing_call, probe.listing_command())
        self.assertTrue(build[1].endswith("run-tests.py"))
        for flag, value in (("--profile", probe.PROFILE), ("--phase", "build"), ("--runner", probe.HOST),
                            ("--lane", probe.LANE)):
            self.assertEqual(build[build.index(flag) + 1], value)
        self.assertTrue(lookup[1].endswith("rust_inventory_baseline.py"))
        self.assertEqual(lookup[lookup.index("--profile") + 1], probe.PROFILE)

    def test_a_failed_archive_or_listing_stops_before_accounting(self) -> None:
        problems, calls = self.walk(build=1)
        self.assertEqual(len(calls), 1)
        self.assertIn("не собран", problems[0])

    def test_a_failed_ordinary_listing_is_a_refusal(self) -> None:
        problems, _ = self.walk(listed=101)
        self.assertIn(f"{probe.HOST}/{probe.LISTED}", problems[0])

    def test_a_failed_lookup_of_the_night_is_a_refusal(self) -> None:
        problems, _ = self.walk(lookup=1)
        self.assertIn("инвентарь прошлой ночи не найден", problems[0])

    def test_the_lane_option_reaches_the_night_builder(self) -> None:
        spec = importlib.util.spec_from_file_location("probe_run_tests", ROOT / "scripts" / "ci" / "run-tests.py")
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        with mock.patch.object(lanes, "build_host", return_value=0) as built:
            self.assertEqual(runner.main(["--profile", "large", "--ecosystem", "rust", "--phase", "build",
                                          "--prepared", "p", "--results", "r", "--runner", probe.HOST,
                                          "--lane", probe.LANE]), 0)
        self.assertEqual(built.call_args.args[-1], probe.LANE)
        with self.assertRaises(SystemExit):
            runner.main(["--profile", "large", "--ecosystem", "rust", "--phase", "verify", "--results", "r",
                         "--sha", SHA, "--lane", probe.LANE])


class BuildHostTests(unittest.TestCase):
    def test_one_lane_builds_only_that_lane_and_refuses_lanes_outside_the_plan(self) -> None:
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(lanes, "build") as built:
            root = Path(temporary)
            self.assertEqual(lanes.build_host(root, root / "w", root / "r", "large", probe.HOST, None, probe.LANE), 0)
            self.assertEqual([call.args[2] for call in built.call_args_list], [probe.LANE])
            with self.assertRaisesRegex(ValueError, "not in the queue plan"):
                lanes.build_host(root, root / "w", root / "r", "queue", probe.HOST, None, "vendor")


class WorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        sys.path.insert(0, str(ROOT))
        from tests.ci.test_unica_workflow import evaluate, step_by_id

        self.evaluate = evaluate
        self.gate = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"))["jobs"]["unica-ci"]
        self.step = step_by_id(self.gate, "nightly-path")

    def test_the_queue_walks_the_nightly_path_with_read_access_to_the_night_inventory(self) -> None:
        self.assertIn("nightly-path-probe.py", self.step["run"])
        self.assertEqual(self.step["env"]["BASE_SHA"], "${{ github.event.merge_group.base_sha }}")
        self.assertEqual(self.step["env"]["HEAD_SHA"], "${{ github.event.merge_group.head_sha }}")
        self.assertEqual(self.step["env"]["GH_TOKEN"], "${{ github.token }}")
        self.assertEqual(self.gate["permissions"].get("actions"), "read")
        for event in ("merge_group", "pull_request"):
            for kept in ("true", "false"):
                with self.subTest(event=event, kept=kept):
                    values = {"github.event_name": event, "env.KEPT": kept}
                    self.assertEqual(self.evaluate(self.step["if"], values), event == "merge_group" and kept == "false")
        # Красный шаг выше не отменяет проверку: обе поломки видны в одном прогоне.
        self.assertTrue(self.evaluate(self.step["if"], {"github.event_name": "merge_group", "env.KEPT": "false",
                                                        "failure": True}))


if __name__ == "__main__":
    unittest.main()
