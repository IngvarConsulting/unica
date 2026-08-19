"""Guards for the cold-start measurement harness.

The harness exists to accept a gate — "core-only cold start under fifteen
seconds at half a megabyte per second" — so its own numbers have to be worth
believing. Two properties carry that: the same scenario measured twice agrees,
and the breakdown accounts for the whole wall clock rather than a chosen slice
of it. A harness that drifts between runs cannot refuse a phase, and one whose
steps do not add up hides the cost somewhere the reader will not look.

Measurement runs against a local throttled server rather than the real network:
the point is a scenario reproducible on any machine, not a speed test of the
one it runs on.
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "measure-cold-start.py"
RUNTIME = REPO_ROOT / "target" / "release" / "unica"

SPEC = importlib.util.spec_from_file_location("measure_cold_start", SCRIPT)
HARNESS = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = HARNESS
SPEC.loader.exec_module(HARNESS)

# Small payload, generous rate: the properties under test are shape and
# repeatability, not the numbers a real scenario produces.
PAYLOAD_BYTES = 2 * 1024 * 1024
RATE_BYTES = 8 * 1024 * 1024


def binaries_present() -> bool:
    return RUNTIME.is_file()


class ThrottledServerTests(unittest.TestCase):
    def test_the_server_paces_bytes_at_the_requested_rate(self) -> None:
        """The throttle is the whole reason a run is reproducible."""
        with tempfile.TemporaryDirectory() as raw:
            payload = Path(raw) / "payload.bin"
            payload.write_bytes(b"\0" * (1024 * 1024))
            with HARNESS.serve_throttled(payload, rate=2 * 1024 * 1024) as server:
                elapsed, served = HARNESS.fetch_all(server.url)
        self.assertEqual(served, 1024 * 1024)
        # One megabyte at two per second: half a second, with room for a slow
        # machine but not enough to pass an unthrottled server.
        self.assertGreater(elapsed, 0.35)
        self.assertLess(elapsed, 2.0)

    def test_the_server_supports_range_requests(self) -> None:
        """Phase 2 resumes interrupted downloads, and it will measure here first."""
        with tempfile.TemporaryDirectory() as raw:
            payload = Path(raw) / "payload.bin"
            payload.write_bytes(bytes(range(256)) * 8)
            with HARNESS.serve_throttled(payload, rate=RATE_BYTES) as server:
                tail = HARNESS.fetch_range(server.url, start=1024)
        self.assertEqual(len(tail), 2048 - 1024)
        self.assertEqual(tail[:4], (bytes(range(256)) * 8)[1024:1028])


class ScenarioTests(unittest.TestCase):
    def setUp(self) -> None:
        if not binaries_present():
            self.skipTest("release binaries are not built; run cargo build --release")

    def measure(self) -> dict:
        return HARNESS.measure(
            runtime=RUNTIME,
            payload_bytes=PAYLOAD_BYTES,
            rate=RATE_BYTES,
        )

    def test_the_breakdown_accounts_for_the_whole_wall_clock(self) -> None:
        """A step that hides time is worse than no breakdown at all."""
        result = self.measure()
        steps = ("download", "verify", "extract", "spawn_initialize", "tools_list")
        for step in steps:
            with self.subTest(step=step):
                self.assertIn(step, result)
                self.assertGreaterEqual(result[step], 0.0)
        self.assertAlmostEqual(
            sum(result[step] for step in steps), result["total"], delta=0.25,
            msg="the named steps must add up to the measured total",
        )

    def test_two_runs_of_one_scenario_agree(self) -> None:
        """Without this the harness cannot refuse a phase, only annoy it."""
        first, second = self.measure(), self.measure()
        self.assertAlmostEqual(
            first["download"], second["download"], delta=0.5,
            msg="download is paced by the throttle and must repeat",
        )
        self.assertAlmostEqual(
            first["total"], second["total"], delta=1.5,
            msg="the whole scenario must repeat within noise",
        )

    def test_a_run_reaches_a_real_tools_list(self) -> None:
        """Measuring a handshake that never happened would measure nothing."""
        result = self.measure()
        self.assertGreater(result["tools"], 0, "tools/list must return real tools")
        self.assertEqual(result["server"], "unica")


class OutputTests(unittest.TestCase):
    def setUp(self) -> None:
        if not binaries_present():
            self.skipTest("release binaries are not built; run cargo build --release")

    def test_the_script_writes_one_json_line_per_run(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            out = Path(raw) / "runs.jsonl"
            done = subprocess.run(
                [
                    sys.executable, str(SCRIPT),
                    "--payload", str(PAYLOAD_BYTES),
                    "--rate", str(RATE_BYTES),
                    "--repeat", "2",
                    "--out", str(out),
                ],
                cwd=REPO_ROOT, capture_output=True, text=True,
            )
            self.assertEqual(done.returncode, 0, done.stderr)
            lines = [json.loads(line) for line in out.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(len(lines), 2)
        for line in lines:
            self.assertEqual(line["rate"], RATE_BYTES)
            self.assertIn("total", line)


if __name__ == "__main__":
    unittest.main()
