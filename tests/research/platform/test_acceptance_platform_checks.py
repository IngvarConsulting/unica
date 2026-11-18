"""Acceptance scenarios confirmed by a BSL check on the real 1C platform.

Each declaration in `tests/fixtures/acceptance/platform-checks/` names a
scenario and a BSL module; `tests/ci/acceptance_platform.py` describes the
contract. The test runs the scenario through MCP on a copy of its fixture,
creates an empty file infobase from the result, starts the thin client with
the module and requires its exact `PASS` line in a fresh `result.txt`. A
declared negative control runs the module on the pristine fixture and requires
its rejection.

Research test on the real platform: ordinary runs skip it, and
`scripts/dev/run-research.py --profile platform` runs it by hand. It needs
`UNICA_PLATFORM_BUILD`, the installed platform build such as `8.3.27.2342`.
The product is the one `scripts/ci/prepared_binary.py` builds; the pinned
v8-runner from `plugins/unica/third-party/tools.lock.json` is staged beside it
as its plugin root.
"""
from __future__ import annotations

import os
import unittest
from pathlib import Path

from scripts.ci.prepared_binary import ensure_binary
from scripts.ci.temp_tree import temp_tree
from tests.ci import test_acceptance_scenarios as acceptance
from tests.ci.acceptance_controls import load_acceptance_corpus
from tests.ci.acceptance_platform import load_platform_checks, negative_scenario, platform_driver
from tests.ci.acceptance_profiles import stage_tools

ROOT = Path(__file__).resolve().parents[3]
BUILD_ENV = "UNICA_PLATFORM_BUILD"


@unittest.skipUnless(
    os.environ.get(BUILD_ENV),
    f"нужна установленная платформа 1С: задайте {BUILD_ENV}",
)
class AcceptancePlatformChecks(unittest.TestCase):
    def test_declared_scenarios_pass_their_platform_checks(self) -> None:
        build = os.environ[BUILD_ENV]
        corpus = load_acceptance_corpus()
        checks = load_platform_checks(corpus=corpus)
        self.assertTrue(checks, "no platform check is declared")
        ensure_binary(ROOT)
        with temp_tree("unica-platform-checks-") as raw:
            root = Path(raw).resolve()
            try:
                plugin = stage_tools(root / "plugin", ("v8-runner",))
            except (SystemExit, OSError) as error:
                self.skipTest(f"нет закреплённого v8-runner: {error}")
            for check in checks:
                with self.subTest(scenario=check["scenario"]):
                    results = root / "results" / check["scenario"]
                    acceptance.run_corpus(
                        self, {**corpus, "scenarios": [check["scenarioBody"]]}, plugin,
                        scenario_driver=platform_driver(check, build, results / "scenario"),
                    )
                    if "negativeControl" in check:
                        acceptance.run_corpus(
                            self, {**corpus, "scenarios": [negative_scenario(check)]}, plugin,
                            scenario_driver=platform_driver(
                                check, build, results / "negative", negative=True),
                        )


if __name__ == "__main__":
    unittest.main()
