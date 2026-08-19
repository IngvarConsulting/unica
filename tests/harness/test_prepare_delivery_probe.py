"""Guards for the engine-delivery probe preparation.

The probe itself is manual: a host is watched by a person, not by a suite. What
a test can hold is the part that decides whether the probe is worth starting —
the pre-flight against a release. Two properties carry it. A release without a
separate engine archive must be refused with a distinguishable exit code rather
than sending the operator to watch a session that can only fail. And a release
that does carry one must be accepted, so the refusal is not merely permanent.

The release is not contacted here: the check is a pure function of the asset
list, and reaching GitHub would make the suite depend on the network and on
what happens to be published today.
"""

from __future__ import annotations

import importlib.util
import io
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "prepare-delivery-probe.py"

SPEC = importlib.util.spec_from_file_location("prepare_delivery_probe", SCRIPT)
PROBE = importlib.util.module_from_spec(SPEC)
sys.modules["prepare_delivery_probe"] = PROBE
SPEC.loader.exec_module(PROBE)


def run(assets: "list[str]", *arguments: str) -> "tuple[int, str]":
    captured = io.StringIO()
    with mock.patch.object(PROBE, "release_assets", return_value=("v9.9.9", assets)):
        with mock.patch.object(sys, "argv", ["prepare-delivery-probe.py", *arguments]):
            with redirect_stdout(captured):
                code = PROBE.main()
    return code, captured.getvalue()


class PreFlightTests(unittest.TestCase):
    def setUp(self) -> None:
        self.target = PROBE.host_target()

    def test_a_release_without_an_engine_archive_refuses_the_probe(self) -> None:
        code, output = run([f"unica-runtime-{self.target}.tar.gz"])

        self.assertEqual(code, 2, output)
        self.assertIn("нет отдельного архива движка", output)
        self.assertIn("новым упаковщиком", output)

    def test_a_release_that_carries_the_engine_hands_over_the_script(self) -> None:
        with mock.patch.object(PROBE.Path, "mkdir"):
            code, output = run(
                [
                    f"unica-runtime-{self.target}.tar.gz",
                    f"bsl-analyzer-runtime-{self.target}.tar.gz",
                ]
            )

        self.assertEqual(code, 0, output)
        self.assertIn("io.unica/deliveryProgress", output)
        self.assertIn("unica.code.graph", output)

    def test_the_engine_is_named_by_the_caller(self) -> None:
        with mock.patch.object(PROBE.Path, "mkdir"):
            code, output = run(
                [
                    f"unica-runtime-{self.target}.tar.gz",
                    f"rlm-tools-bsl-runtime-{self.target}.tar.gz",
                ],
                "--engine",
                "rlm-tools-bsl",
            )

        self.assertEqual(code, 0, output)
        self.assertIn("rlm-tools-bsl", output)


if __name__ == "__main__":
    unittest.main()
