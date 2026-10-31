"""Метрики серии #1816 читаются из документов GitHub без запуска прогонов."""
from __future__ import annotations

import csv
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import ci_run_metrics as metrics

REPO = "owner/repo"

MAC_LOG = """\
2026-10-21T02:24:45.9857350Z Current runner version: '2.337.0'
2026-10-21T02:24:45.9870000Z ##[group]Runner Image Provisioner
2026-10-21T02:24:45.9873110Z Version: 20260828.587
2026-10-21T02:24:45.9874000Z ##[endgroup]
2026-10-21T02:24:45.9878000Z ##[group]Runner Image
2026-10-21T02:24:45.9878090Z Image: macos-14-arm64
2026-10-21T02:24:45.9878460Z Version: 20260831.0302.1
2026-10-21T02:24:45.9879000Z ##[endgroup]
2026-10-21T02:24:59.9004480Z   stable-aarch64-apple-darwin updated - rustc 1.99.1 (58cf8f9c3 2026-10-13) (from rustc 1.98.0 (88d9e12ae 2026-08-18))
2026-10-21T02:25:01.4549180Z cargo-nextest 0.9.148 (cd1d6d546 2026-10-07)
2026-10-21T02:25:01.4672030Z ##[group]Run Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6
2026-10-21T02:25:01.4672630Z   lookup-only: true
2026-10-21T02:25:01.4672700Z   shared-key: ci-rust-ordinary
2026-10-21T02:25:01.4673000Z   GATE_PROFILE: queue
2026-10-21T02:25:02.5591540Z No cache found.
2026-10-21T02:25:02.5710760Z ##[group]Run python scripts/ci/run-tests.py --phase build
2026-10-21T02:28:44.0769460Z \x1b[1m\x1b[92m    Finished\x1b[0m `test` profile [unoptimized + debuginfo] target(s) in 3m 26s
2026-10-21T02:28:47.0769460Z \x1b[1m\x1b[92m    Finished\x1b[0m `test` profile [unoptimized + debuginfo] target(s) in 0.44s
"""

BASELINE_LOG = """\
2026-10-20T13:03:44.9403890Z ##[group]Run Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6
2026-10-20T13:03:44.9407270Z   lookup-only: false
2026-10-20T13:03:46.0001100Z Cache hit for: v0-rust-test-rust-platforms-Darwin-arm64-b3d57be6-63456410
2026-10-20T13:04:02.3333090Z Restored from cache key "v0-rust-test-rust-platforms-Darwin-arm64-b3d57be6-63456410" full match: true.
2026-10-20T13:04:02.4061170Z ##[group]Run actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9
2026-10-20T13:04:03.0000000Z Cache restored from key: cargo-macOS-x
2026-10-20T13:04:04.0000000Z ##[group]Run Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6
2026-10-20T13:04:05.0000000Z Restored from cache key "v0-rust-prefix" full match: false.
2026-10-20T13:10:05.3227500Z     Finished `test` profile [unoptimized + debuginfo] target(s) in 65m 55s
"""


def job(identifier, name, created, started, completed, *, label="ubuntu-latest", conclusion="success", steps=()):
    ran = conclusion != "skipped"
    return {"id": identifier, "name": name, "labels": [label], "conclusion": conclusion,
            "runner_name": f"GitHub Actions {identifier}" if ran else "", "created_at": created,
            "started_at": started, "completed_at": completed, "steps": list(steps)}


def step(name, started, completed, conclusion="success"):
    return {"name": name, "started_at": started, "completed_at": completed, "conclusion": conclusion}


def t(clock: str) -> str:
    return f"2026-10-21T{clock}Z"


def queue_run(attempt=1):
    return {"id": 7, "run_attempt": attempt, "event": "merge_group", "head_sha": "a" * 40, "head_branch": "queue",
            "status": "completed", "conclusion": "success", "created_at": t("02:00:00"),
            "run_started_at": t("02:00:00") if attempt == 1 else t("03:00:00"), "html_url": "https://example/7"}


def queue_jobs():
    return [
        job(1, "classify-changes", t("02:00:00"), t("02:00:02"), t("02:00:20")),
        job(2, "Build Rust ordinary (macos-14)", t("02:00:20"), t("02:00:50"), t("02:05:20"), label="macos-14", steps=[
            step("Set up job", t("02:00:50"), t("02:00:53")),
            step("Run Swatinem/rust-cache@x", t("02:00:53"), t("02:00:55")),
            step("Run python scripts/ci/run-tests.py --phase build", t("02:00:55"), t("02:05:00")),
            step("Run actions/upload-artifact@x", t("02:05:00"), t("02:05:15")),
            step("Post Run Swatinem/rust-cache@x", t("02:05:15"), t("02:05:16")),
            step("Run skipped thing", t("02:05:16"), t("02:05:19"), conclusion="skipped"),
        ]),
        job(3, "Rust ordinary (macos-14, 1/2)", t("02:05:20"), t("02:05:26"), t("02:11:26"), label="macos-14", steps=[
            step("Run actions/download-artifact@x", t("02:05:30"), t("02:05:56")),
            step("Кэш зависимостей для Clippy", t("02:05:56"), t("02:06:00")),
        ]),
        job(4, "Rust tests (complete inventory)", t("02:11:26"), t("02:11:28"), t("02:11:40")),
        job(5, "Unica CI", t("02:11:40"), t("02:11:42"), t("02:11:50")),
        job(6, "Build tools (${{ matrix.target }})", t("02:11:40"), t("02:11:40"), t("02:11:40"), conclusion="skipped"),
    ]


def zipped(member: str, document: dict) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr(member, json.dumps(document))
    return buffer.getvalue()


def lane_document(attempt="1"):
    return {"runner": "macos-14", "lane": "ordinary", "profile": "queue", "buildSeconds": 221.9, "listSeconds": 3.4,
            "archiveBytes": 224066134, "inventory": [{}] * 3, "shards": [[{}], [{}, {}]], "attempt": attempt,
            "identity": {"sha": "a" * 40, "target": "aarch64-apple-darwin", "toolchain": "rustc 1.99.1 (58cf8f9c3 2026-10-13)\nbinary: rustc"},
            "nextestVersion": "cargo-nextest 0.9.148 (cd1d6d546 2026-10-07)\nrelease: 0.9.148"}


def shard_document(attempt="1"):
    return {"runner": "macos-14", "lane": "ordinary", "shard": 1, "executionSeconds": 300.5, "exitCode": 0,
            "plan": [{}, {}], "attempt": attempt}


class FakeApi:
    """Ответы `gh api` по пути; считает обращения, чтобы проверить локальный кеш."""

    def __init__(self, responses: dict[str, bytes]) -> None:
        self.responses = responses
        self.calls: list[str] = []

    def __call__(self, path: str) -> bytes:
        self.calls.append(path)
        if path not in self.responses:
            raise RuntimeError(f"HTTP 404: {path}")
        return self.responses[path]


def scripted(*outcomes):
    """`gh api`, отвечающий по очереди: исключение бросается, байты возвращаются."""
    pending = list(outcomes)

    def api(path):
        outcome = pending.pop(0)
        if isinstance(outcome, Exception):
            raise outcome
        return outcome

    return api


def queue_api(run=None, jobs=None, artifacts=None) -> FakeApi:
    run = run or queue_run()
    jobs = queue_jobs() if jobs is None else jobs
    base = f"repos/{REPO}/actions"
    artifacts = artifacts if artifacts is not None else [
        {"id": 11, "name": "plan-rust-macos-14-ordinary", "expired": False},
        {"id": 12, "name": "results-rust-macos-14-ordinary-1", "expired": False},
        {"id": 13, "name": "results-rust-macos-14-ordinary-2", "expired": True},
        {"id": 14, "name": "results-python-ci-small", "expired": False},
    ]
    responses = {
        f"{base}/runs/7": json.dumps(run).encode(),
        f"{base}/runs/7/attempts/{run['run_attempt']}": json.dumps(run).encode(),
        f"{base}/runs/7/attempts/{run['run_attempt']}/jobs?per_page=100&page=1":
            json.dumps({"total_count": len(jobs), "jobs": jobs}).encode(),
        f"{base}/runs/7/artifacts?per_page=100&page=1":
            json.dumps({"total_count": len(artifacts), "artifacts": artifacts}).encode(),
        f"{base}/artifacts/11/zip": zipped("lane.json", lane_document()),
        f"{base}/artifacts/12/zip": zipped("shard.json", shard_document()),
    }
    for item in jobs:
        responses[f"{base}/jobs/{item['id']}/logs"] = MAC_LOG.encode()
    return FakeApi(responses)


class LogParsingTests(unittest.TestCase):
    def test_runner_image_toolchain_nextest_profile_and_compile_lines_come_from_the_log(self):
        parsed = metrics.parse_log(MAC_LOG)
        self.assertEqual(parsed["image"], {"image": "macos-14-arm64", "version": "20260831.0302.1"})
        self.assertEqual(parsed["toolchain"], "1.99.1 58cf8f9c3 2026-10-13")
        self.assertEqual(parsed["nextest"], "0.9.148")
        self.assertEqual(parsed["profile"], "queue")
        self.assertEqual(parsed["cargoFinishedSeconds"], [206.0, 0.44])

    def test_lookup_only_miss_is_not_a_restore(self):
        [cache] = metrics.parse_log(MAC_LOG)["cache"]
        self.assertEqual(cache, {"action": "rust-cache", "lookupOnly": True, "key": None, "match": "none", "restored": False})

    def test_exact_partial_and_actions_cache_restores_are_distinguished(self):
        parsed = metrics.parse_log(BASELINE_LOG)
        self.assertEqual([(entry["action"], entry["match"], entry["restored"]) for entry in parsed["cache"]],
                         [("rust-cache", "exact", True), ("actions/cache", "exact", True), ("rust-cache", "partial", True)])
        self.assertEqual(parsed["cache"][0]["key"], "v0-rust-test-rust-platforms-Darwin-arm64-b3d57be6-63456410")
        self.assertEqual(parsed["cargoFinishedSeconds"], [3955.0])
        self.assertIsNone(parsed["image"])

    def test_lookup_only_hits_are_found_but_not_restored_and_restore_key_hits_stay_partial(self):
        log = """\
##[group]Run Swatinem/rust-cache@x
  lookup-only: true
Cache hit for: v0-exact
Found cache key "v0-exact" full match: true.
##[group]Run Swatinem/rust-cache@x
  lookup-only: true
Cache hit for restore-key: v0-prefix-old
Found cache key "v0-prefix-old" full match: false.
##[group]Run actions/cache@x
Cache hit for restore-key: cargo-old
Cache restored from key: cargo-old
"""
        self.assertEqual([(entry["lookupOnly"], entry["match"], entry["restored"], entry["key"])
                          for entry in metrics.parse_log(log)["cache"]],
                         [(True, "exact", False, "v0-exact"), (True, "partial", False, "v0-prefix-old"),
                          (False, "partial", True, "cargo-old")])
        jobs = [job(2, "Build Rust ordinary (macos-14)", t("02:00:00"), t("02:00:01"), t("02:01:00"))]
        counted = metrics.run_metrics(queue_run(), jobs, {2: metrics.parse_log(log)}, [], [])["cacheRestored"]
        self.assertEqual(counted, {"exact": 0, "partial": 1, "lookupOnly": 2, "miss": 0})

    def test_cargo_durations_in_every_format(self):
        for text, seconds in (("17.43s", 17.43), ("3m 26s", 206.0), ("1h 2m 05s", 3725.0)):
            with self.subTest(text=text):
                self.assertAlmostEqual(metrics.cargo_seconds(text), seconds)

    def test_steps_fall_into_setup_cache_transfer_and_work(self):
        cases = {
            "Set up job": "setup", "Complete job": "setup", "Run actions/checkout@x": "setup",
            "Run dtolnay/rust-toolchain@x": "setup", "Post Run actions/checkout@x": "setup",
            "Run Swatinem/rust-cache@x": "cache", "Post Run Swatinem/rust-cache@x": "cache",
            "Кэш зависимостей для Clippy": "cache", "Restore Cargo target cache": "cache",
            "Run actions/upload-artifact@x": "transfer", "Run actions/download-artifact@x": "transfer",
            "Run python scripts/ci/run-tests.py --phase run": "work", "Run set -o pipefail": "work",
        }
        for name, phase in cases.items():
            with self.subTest(name=name):
                self.assertEqual(metrics.step_phase(name), phase)


class RunMetricsTests(unittest.TestCase):
    def test_gate_time_runner_minutes_and_wait_come_from_executed_jobs(self):
        result = metrics.run_metrics(queue_run(), queue_jobs(), {}, [], [])
        self.assertEqual(result["unicaCi"]["seconds"], 710.0)
        self.assertEqual(result["gateEquivalentSeconds"], 700.0)
        self.assertEqual(result["lastGateJob"], "Rust tests (complete inventory)")
        self.assertFalse(result["deliveryTailRan"])
        self.assertTrue(result["rustTestsRan"])
        self.assertFalse(metrics.run_metrics(queue_run(), queue_jobs()[:1] + queue_jobs()[3:], {}, [], [])["rustTestsRan"])
        self.assertAlmostEqual(result["runnerMinutes"]["macos"], (270 + 360) / 60)
        self.assertAlmostEqual(result["runnerMinutes"]["linux"], (18 + 12 + 8) / 60)
        self.assertAlmostEqual(result["runnerMinutes"]["total"], (270 + 360 + 38) / 60)
        self.assertEqual(result["runnerWait"]["maxSeconds"], 30.0)
        self.assertEqual(result["runnerWait"]["macosMaxSeconds"], 30.0)
        self.assertEqual(result["phaseSeconds"], {"setup": 3.0, "cache": 7.0, "work": 245.0, "transfer": 41.0})

    def test_delivery_tail_extends_unica_ci_but_not_the_comparable_gate(self):
        jobs = queue_jobs()
        jobs[-1] = job(6, "Build tools (linux-x64)", t("02:11:30"), t("02:11:31"), t("02:30:00"))
        jobs[-2] = job(5, "Unica CI", t("02:30:00"), t("02:30:02"), t("02:30:10"))
        result = metrics.run_metrics(queue_run(), jobs, {}, [], [])
        self.assertTrue(result["deliveryTailRan"])
        self.assertEqual(result["unicaCi"]["seconds"], 1810.0)
        self.assertEqual(result["gateEquivalentSeconds"], 700.0)

    def test_jobs_carried_over_from_a_previous_attempt_are_not_counted(self):
        jobs = queue_jobs()
        for item in jobs:
            item["run_attempt"] = 2
        jobs[1]["run_attempt"] = 1
        result = metrics.run_metrics(queue_run(attempt=2), jobs, {}, [], [])
        self.assertAlmostEqual(result["runnerMinutes"].get("macos", 0.0), 360 / 60)

    def test_a_rerun_attempt_is_measured_from_its_own_start(self):
        jobs = [job(5, "Unica CI", t("03:10:00"), t("03:10:01"), t("03:10:10"))]
        result = metrics.run_metrics(queue_run(attempt=2), jobs, {}, [], [])
        self.assertEqual(result["origin"], t("03:00:00"))
        self.assertEqual(result["unicaCi"]["seconds"], 610.0)

    def test_log_identity_and_cache_are_aggregated_per_run(self):
        logs = {2: metrics.parse_log(MAC_LOG), 3: metrics.parse_log(BASELINE_LOG)}
        result = metrics.run_metrics(queue_run(), queue_jobs(), logs, [metrics.lane_metrics(lane_document())], [])
        self.assertEqual(result["identity"]["images"], ["macos-14-arm64 20260831.0302.1"])
        self.assertEqual(result["identity"]["toolchains"], ["1.99.1 58cf8f9c3 2026-10-13"])
        self.assertEqual(result["identity"]["targets"], ["aarch64-apple-darwin"])
        self.assertEqual(result["cacheRestored"], {"exact": 2, "partial": 1, "lookupOnly": 1, "miss": 1})
        self.assertEqual(result["cargoFinished"]["count"], 3)
        self.assertEqual(result["laneTotals"]["buildSecondsMax"], 221.9)


class SummaryTests(unittest.TestCase):
    @staticmethod
    def runs(label, gates):
        return [{"label": label, "runId": 1, "conclusion": "success", "event": "workflow_dispatch", "deliveryTailRan": False,
                 "identity": {key: [] for key in ("images", "toolchains", "nextest", "targets", "profiles")},
                 "unicaCi": {"seconds": gate}, "gateEquivalentSeconds": gate - 10, "runnerMinutes": {"total": 60.0},
                 "runnerWait": {"maxSeconds": 5.0}, "phaseSeconds": {}, "cargoFinished": {"sumSeconds": 0.0},
                 "laneTotals": {"buildSecondsMax": None, "executionSecondsMax": None}} for gate in gates]

    def test_p95_is_the_observed_nearest_rank(self):
        self.assertEqual(metrics.nearest_rank(list(range(1, 11)), 0.95), 10)
        self.assertEqual(metrics.nearest_rank(list(range(1, 21)), 0.95), 19)
        self.assertEqual(metrics.nearest_rank([5.0], 0.95), 5.0)

    def test_groups_get_median_and_p95_and_pairs_get_deltas(self):
        summary = metrics.summarize(self.runs("before", [1400, 1500, 1300, 1600]) + self.runs("after", [600, 800, 700, 1000]),
                                    ("before", "after"))
        before = summary["groups"]["before"]["metrics"]["unicaCiSeconds"]
        self.assertEqual((before["n"], before["p50"], before["p95"]), (4, 1450.0, 1600))
        self.assertIsNone(summary["groups"]["after"]["metrics"]["laneBuildSecondsMax"])
        delta = summary["pairedDelta"]["metrics"]["unicaCiSeconds"]
        self.assertEqual((delta["n"], delta["p50"], delta["max"]), (4, -650.0, -600))

    def test_failed_runs_and_their_pairs_stay_out_of_the_statistics(self):
        runs = self.runs("before", [1400, 1500, 1300]) + self.runs("after", [600, 100, 700])
        for run, conclusion in zip(runs, ("success",) * 4 + ("failure", "success")):
            run.update(conclusion=conclusion, runId=len(conclusion))
        summary = metrics.summarize(runs, ("before", "after"))
        self.assertEqual(summary["groups"]["after"]["runs"], 2)
        self.assertEqual(summary["groups"]["after"]["excluded"], [{"runId": 7, "reason": "conclusion failure"}])
        self.assertEqual(summary["groups"]["after"]["metrics"]["unicaCiSeconds"]["p50"], 650.0)
        delta = summary["pairedDelta"]
        self.assertEqual((delta["pairs"], delta["droppedPairs"]), (2, 1))
        self.assertEqual(delta["metrics"]["unicaCiSeconds"]["p50"], -700.0)
        included = metrics.summarize(runs, ("before", "after"), include_failed=True)
        self.assertEqual(included["groups"]["after"]["runs"], 3)

    def test_a_run_without_rust_tests_is_not_comparable(self):
        runs = self.runs("after", [700, 190])
        runs[1].update(rustTestsRan=False, runId=9)
        summary = metrics.summarize(runs)
        self.assertEqual(summary["groups"]["after"]["excluded"], [{"runId": 9, "reason": "без Rust-тестов"}])
        self.assertEqual(summary["groups"]["after"]["metrics"]["unicaCiSeconds"]["n"], 1)
        runs[1]["conclusion"] = "failure"
        included = metrics.summarize(runs, include_failed=True)
        self.assertEqual(included["groups"]["after"]["excluded"], [{"runId": 9, "reason": "без Rust-тестов"}])

    def test_unequal_pairs_are_refused(self):
        with self.assertRaises(ValueError):
            metrics.summarize(self.runs("before", [1, 2]) + self.runs("after", [1]), ("before", "after"))


def two_runner_jobs():
    """Раскладка #1813: Linux целиком в `Unica CI`, macOS — в `Rust (macos-14)`."""
    return [
        job(1, "Unica CI", t("02:00:00"), t("02:00:10"), t("02:20:00")),
        job(2, "Rust (macos-14)", t("02:00:00"), t("02:00:30"), t("02:08:00"), label="macos-14"),
        job(3, "Build tools (${{ matrix.target }})", t("02:20:00"), t("02:20:00"), t("02:20:00"), conclusion="skipped"),
        job(4, "Delivery CI", t("02:20:00"), t("02:20:00"), t("02:20:00"), conclusion="skipped"),
    ]


class TwoRunnerLayoutTests(unittest.TestCase):
    def test_the_gate_job_is_the_linux_work_and_counts_toward_the_comparable_time(self):
        result = metrics.run_metrics(queue_run(), two_runner_jobs(), {}, [], [metrics.shard_metrics(shard_document())])
        self.assertEqual(result["unicaCi"]["seconds"], 1200.0)
        self.assertEqual(result["gateEquivalentSeconds"], 1200.0)
        self.assertEqual(result["lastGateJob"], "Unica CI")
        self.assertFalse(result["deliveryTailRan"])
        # Имя джобы больше не говорит, исполнялся ли Rust: это видно по частям.
        self.assertTrue(result["rustTestsRan"])
        self.assertFalse(metrics.run_metrics(queue_run(), two_runner_jobs(), {}, [], [])["rustTestsRan"])
        self.assertAlmostEqual(result["runnerMinutes"]["linux"], 1190 / 60)
        self.assertAlmostEqual(result["runnerMinutes"]["macos"], 450 / 60)
        self.assertEqual(result["runnerWait"]["macosMaxSeconds"], 30.0)

    def test_the_delivery_tail_with_its_gate_stays_out_of_the_comparable_time(self):
        jobs = two_runner_jobs()
        jobs[2] = job(3, "Build tools (linux-x64)", t("02:20:00"), t("02:20:05"), t("02:40:00"))
        jobs[3] = job(4, "Delivery CI", t("02:40:00"), t("02:40:05"), t("02:40:20"))
        result = metrics.run_metrics(queue_run(), jobs, {}, [], [])
        self.assertTrue(result["deliveryTailRan"])
        self.assertEqual(result["gateEquivalentSeconds"], 1200.0)
        self.assertEqual(result["lastGateJob"], "Unica CI")

    def test_current_macos15_artifacts_are_read_without_retagging_historical_macos14(self):
        base = f"repos/{REPO}/actions"
        api = queue_api(jobs=two_runner_jobs(), artifacts=[
            {"id": 25, "name": "plan-macos-15", "expired": False},
            {"id": 26, "name": "results-macos-15", "expired": False},
        ])
        for identifier, name, document in [(25, "plan-rust-macos-15-ordinary/lane.json", lane_document()),
                                            (26, "results-rust-macos-15-ordinary-1/shard.json", shard_document())]:
            payload = io.BytesIO()
            with zipfile.ZipFile(payload, "w") as archive:
                archive.writestr(name, json.dumps({**document, "runner": "macos-15"}))
            api.responses[f"{base}/artifacts/{identifier}/zip"] = payload.getvalue()
        result = metrics.collect(metrics.GitHub(REPO, None, api), 7, None, logs=False, artifacts=True)
        self.assertEqual([item["runner"] for item in result["lanes"]], ["macos-15"])
        self.assertEqual([item["runner"] for item in result["shards"]], ["macos-15"])
        self.assertEqual(lane_document()["runner"], "macos-14")

    def test_lane_directories_inside_runner_artifacts_are_read(self):
        base = f"repos/{REPO}/actions"
        artifacts = [
            {"id": 21, "name": "plan-macos-14", "expired": False},
            {"id": 22, "name": "results-ubuntu-latest", "expired": False},
            {"id": 23, "name": "results-ci-routing", "expired": False},
            {"id": 24, "name": "plan-ubuntu-latest", "expired": True},
        ]
        api = queue_api(jobs=two_runner_jobs(), artifacts=artifacts)
        plan = io.BytesIO()
        with zipfile.ZipFile(plan, "w") as archive:
            archive.writestr("plan-rust-macos-14-ordinary/lane.json", json.dumps(dict(lane_document(), lane="ordinary", shards=[[{}]])))
            archive.writestr("plan-rust-macos-14-platform/plan.json", json.dumps([]))
        results = io.BytesIO()
        with zipfile.ZipFile(results, "w") as archive:
            archive.writestr("results-rust-ubuntu-latest-ordinary-1/shard.json",
                             json.dumps(dict(shard_document(), runner="ubuntu-latest")))
            archive.writestr("results-rust-ubuntu-latest-receipt-1/shard.json",
                             json.dumps(dict(shard_document(), runner="ubuntu-latest", lane="receipt", executionSeconds=60.0)))
            # Python-результаты в том же артефакте частями полос не являются.
            archive.writestr("results-python-ci-small/run.json", json.dumps({"runner": "ubuntu-latest"}))
        api.responses[f"{base}/artifacts/21/zip"] = plan.getvalue()
        api.responses[f"{base}/artifacts/22/zip"] = results.getvalue()
        result = metrics.collect(metrics.GitHub(REPO, None, api), 7, None, logs=True, artifacts=True)
        self.assertEqual([(lane["runner"], lane["lane"], lane["shards"]) for lane in result["lanes"]], [("macos-14", "ordinary", 1)])
        self.assertEqual([(shard["runner"], shard["lane"], shard["executionSeconds"]) for shard in result["shards"]],
                         [("ubuntu-latest", "ordinary", 300.5), ("ubuntu-latest", "receipt", 60.0)])
        self.assertNotIn(f"{base}/artifacts/23/zip", api.calls)
        self.assertNotIn(f"{base}/artifacts/24/zip", api.calls)
        self.assertTrue(result["rustTestsRan"])
        logged = sorted(path for path in api.calls if path.endswith("/logs"))
        self.assertEqual(logged, [f"{base}/jobs/{n}/logs" for n in (1, 2)])


class CollectTests(unittest.TestCase):
    def test_lane_and_shard_artifacts_of_the_attempt_are_read_and_expired_ones_skipped(self):
        api = queue_api()
        result = metrics.collect(metrics.GitHub(REPO, None, api), 7, None, logs=True, artifacts=True)
        self.assertEqual([(lane["lane"], lane["buildSeconds"], lane["tests"], lane["shards"]) for lane in result["lanes"]],
                         [("ordinary", 221.9, 3, 2)])
        self.assertEqual([(shard["shard"], shard["executionSeconds"]) for shard in result["shards"]], [(1, 300.5)])
        self.assertNotIn(f"repos/{REPO}/actions/artifacts/13/zip", api.calls)
        self.assertNotIn(f"repos/{REPO}/actions/artifacts/14/zip", api.calls)
        # Логи читаются только у исполнявшихся Rust-джоб; `Unica CI` с #1813 сам
        # собирает Linux, поэтому его лог читается и в прежней раскладке.
        logged = sorted(path for path in api.calls if path.endswith("/logs"))
        self.assertEqual(logged, [f"repos/{REPO}/actions/jobs/{n}/logs" for n in (2, 3, 4, 5)])

    def test_artifacts_left_from_another_attempt_are_dropped(self):
        api = queue_api(run=queue_run(attempt=2))
        api.responses[f"repos/{REPO}/actions/artifacts/11/zip"] = zipped("lane.json", lane_document(attempt="1"))
        api.responses[f"repos/{REPO}/actions/artifacts/12/zip"] = zipped("shard.json", shard_document(attempt="2"))
        result = metrics.collect(metrics.GitHub(REPO, None, api), 7, 2, logs=False, artifacts=True)
        self.assertEqual(result["lanes"], [])
        self.assertEqual(len(result["shards"]), 1)

    def test_an_unfinished_run_is_refused(self):
        run = dict(queue_run(), status="in_progress")
        with self.assertRaisesRegex(ValueError, "не завершён"):
            metrics.collect(metrics.GitHub(REPO, None, queue_api(run=run)), 7, None, logs=False, artifacts=False)

    def test_jobs_are_paged_and_completed_responses_are_cached_but_the_run_is_reread(self):
        jobs = queue_jobs()
        api = queue_api()
        base = f"repos/{REPO}/actions/runs/7/attempts/1/jobs?per_page=100"
        api.responses[f"{base}&page=1"] = json.dumps({"total_count": len(jobs), "jobs": jobs[:4]}).encode()
        api.responses[f"{base}&page=2"] = json.dumps({"total_count": len(jobs), "jobs": jobs[4:]}).encode()
        with tempfile.TemporaryDirectory() as cache:
            github = metrics.GitHub(REPO, Path(cache), api)
            first = metrics.collect(github, 7, None, logs=True, artifacts=True)
            calls = len(api.calls)
            second = metrics.collect(github, 7, None, logs=True, artifacts=True)
        self.assertEqual(len(first["jobs"]), len(jobs))
        self.assertEqual(first["gateEquivalentSeconds"], second["gateEquivalentSeconds"])
        # Перечитываются только сам прогон и список артефактов: они меняются с попыткой.
        self.assertEqual(api.calls[calls:], [f"repos/{REPO}/actions/runs/7",
                                             f"repos/{REPO}/actions/runs/7/artifacts?per_page=100&page=1"])


class TransientFailureTests(unittest.TestCase):
    def test_transient_failures_are_retried_and_not_found_is_not(self):
        slept: list[float] = []
        flaky = scripted(RuntimeError("TLS handshake timeout"), RuntimeError("HTTP 502"), b"{}")
        self.assertEqual(metrics.GitHub(REPO, None, flaky, sleep=slept.append).get("x"), b"{}")
        self.assertEqual(slept, [2.0, 4.0])
        missing = FakeApi({})
        with self.assertRaisesRegex(RuntimeError, "HTTP 404"):
            metrics.GitHub(REPO, None, missing, sleep=slept.append).get("y")
        self.assertEqual(missing.calls, ["y"])
        self.assertEqual(len(slept), 2)

    def test_an_expired_log_leaves_the_run_in_the_series_without_log_facts(self):
        api = queue_api()
        del api.responses[f"repos/{REPO}/actions/jobs/2/logs"]
        result = metrics.collect(metrics.GitHub(REPO, None, api), 7, None, logs=True, artifacts=False)
        self.assertIsNone(next(item for item in result["jobs"] if item["id"] == 2)["log"])
        self.assertIsNotNone(next(item for item in result["jobs"] if item["id"] == 3)["log"])

    def test_rate_limits_are_retried_and_only_a_missing_log_becomes_none(self):
        limited = scripted(RuntimeError("HTTP 429: rate limit"), RuntimeError("HTTP 403: secondary rate limit"), b"log")
        self.assertEqual(metrics.GitHub(REPO, None, limited, sleep=lambda _: None).log(1), "log")
        for status in ("HTTP 410: gone", "HTTP 404: not found"):
            with self.subTest(status=status):
                self.assertIsNone(metrics.GitHub(REPO, None, scripted(RuntimeError(status)), sleep=lambda _: None).log(1))
        exhausted = scripted(RuntimeError("HTTP 429"), RuntimeError("HTTP 429"))
        with self.assertRaisesRegex(RuntimeError, "HTTP 429"):
            metrics.GitHub(REPO, None, exhausted, sleep=lambda _: None, attempts=2).log(1)

    def test_a_persistent_failure_is_raised_after_the_last_attempt(self):
        broken = scripted(*[RuntimeError("HTTP 500")] * 3)
        with self.assertRaisesRegex(RuntimeError, "HTTP 500"):
            metrics.GitHub(REPO, None, broken, sleep=lambda _: None, attempts=3).get("z")


class CommandLineTests(unittest.TestCase):
    def test_run_arguments_carry_label_and_attempt(self):
        self.assertEqual(metrics.parse_run_argument("39532820676"), ("all", 39532820676, None))
        self.assertEqual(metrics.parse_run_argument("before=39427121659:2"), ("before", 39427121659, 2))
        for bad in ("before=", "x=12:a", "abc"):
            with self.subTest(bad=bad), self.assertRaises(Exception):
                metrics.parse_run_argument(bad)

    def test_main_writes_json_and_tsv_rows(self):
        api = queue_api()
        with tempfile.TemporaryDirectory() as temporary, patch.object(metrics.GitHub, "_gh", staticmethod(api)), \
                patch("sys.stdout", new_callable=io.StringIO) as stdout:
            out = Path(temporary)
            code = metrics.main(["--repo", REPO, "--json", str(out / "m.json"), "--tsv", str(out / "m.tsv"), "after=7"])
            document = json.loads((out / "m.json").read_text(encoding="utf-8"))
            with (out / "m.tsv").open(encoding="utf-8") as stream:
                rows = list(csv.DictReader(stream, delimiter="\t"))
        self.assertEqual(code, 0)
        self.assertEqual(document["summary"]["groups"]["after"]["runs"], 1)
        self.assertEqual(rows[0]["label"], "after")
        self.assertEqual(rows[0]["gateEquivalentSeconds"], "700.0")
        self.assertIn("after: 1 прогонов", stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
