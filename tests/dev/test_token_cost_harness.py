"""Wire-cut token accounting for the #479 «токены на решение» metric.

The harness is a measuring instrument, not a gate. Two properties make its
numbers attributable to a surface change rather than to the run that produced
them, and both are tested here without a built binary:

* the total equals the sum of its parts, so a delta can always be traced to
  the frames that moved;
* the report carries nothing that varies between two runs of the same binary
  on the same episode — no durations, no timestamps, no machine paths — so a
  diff of two reports is a diff of the surface.

A failed call is counted, never discarded: the metric charges for re-guessing,
so a wrong first call and its retry both appear in the total.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/dev/measure-token-cost.py"
SPEC = importlib.util.spec_from_file_location("measure_token_cost", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)

NONDETERMINISTIC_KEYS = (
    "duration",
    "elapsed",
    "timestamp",
    "started",
    "finished",
    "seconds",
    "ms",
)


def sample_calls() -> list[dict]:
    return [
        {
            "tool": "unica.project.map",
            "request": '{"name":"unica.project.map","arguments":{}}',
            "result": '{"ok":true,"data":{"sourceSets":[]}}',
            "is_error": False,
        },
        {
            "tool": "unica.meta.info",
            "request": '{"name":"unica.meta.info","arguments":{"metadataPath":"Catalog.Валюты"}}',
            "result": '{"ok":false,"errors":["target_not_found"]}',
            "is_error": True,
        },
    ]


class TokenizerTests(unittest.TestCase):
    def test_bytes_tokenizer_counts_utf8_bytes(self) -> None:
        """The dependency-free unit is the byte, so CI needs no tiktoken."""
        self.assertEqual(MODULE.count_tokens("Валюты", "bytes"), 12)
        self.assertEqual(MODULE.count_tokens("ok", "bytes"), 2)

    def test_o200k_without_tiktoken_names_the_remedy(self) -> None:
        with mock.patch.dict(sys.modules, {"tiktoken": None}):
            with self.assertRaises(SystemExit) as raised:
                MODULE.count_tokens("Валюты", "o200k_base")
        self.assertIn("tiktoken", str(raised.exception))

    def test_unknown_tokenizer_is_rejected(self) -> None:
        with self.assertRaises(SystemExit):
            MODULE.count_tokens("Валюты", "gpt-2")


class ReportAccountingTests(unittest.TestCase):
    def build(self) -> dict:
        return MODULE.build_report(
            discovery='{"tools":[{"name":"unica.project.map"}]}',
            calls=sample_calls(),
            tokenizer="bytes",
        )

    def test_total_is_the_sum_of_discovery_and_calls(self) -> None:
        report = self.build()
        self.assertEqual(
            report["total_tokens"],
            report["discovery_tokens"]
            + sum(c["request_tokens"] + c["result_tokens"] for c in report["calls"]),
        )
        self.assertGreater(report["discovery_tokens"], 0)

    def test_every_call_is_reported_including_failures(self) -> None:
        report = self.build()
        self.assertEqual([c["tool"] for c in report["calls"]],
                         ["unica.project.map", "unica.meta.info"])
        self.assertEqual([c["is_error"] for c in report["calls"]], [False, True])
        failed = report["calls"][1]
        self.assertGreater(failed["result_tokens"], 0,
                           "a failed call still costs its result tokens")

    def test_report_is_reproducible_for_the_same_input(self) -> None:
        self.assertEqual(self.build(), self.build())

    def test_report_carries_no_nondeterministic_fields(self) -> None:
        serialized = json.dumps(self.build())
        for key in NONDETERMINISTIC_KEYS:
            self.assertNotIn(f'"{key}', serialized,
                             f"report must not carry a {key} field")

    def test_report_names_its_tokenizer(self) -> None:
        self.assertEqual(self.build()["tokenizer"], "bytes")


if __name__ == "__main__":
    unittest.main()
