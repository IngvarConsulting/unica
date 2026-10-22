"""Local XDTO Codex acceptance against an explicitly selected immutable binary."""
import json
import os
import unittest
from pathlib import Path
from unittest.mock import patch

from tests.agent_evaluation import xdto_driver
from tests.ci import test_acceptance_scenarios as acceptance
from tests.ci.acceptance_controls import load_acceptance_corpus
from tests.ci.acceptance_profiles import select_profile


class XdtoAgentAcceptanceTests(unittest.TestCase):
    def test_agent_uses_contextual_contract_and_preserves_package_resources(self):
        binary_text = os.environ.get("UNICA_XDTO_AGENT_BINARY")
        if not binary_text:
            self.fail("set UNICA_XDTO_AGENT_BINARY to the immutable candidate binary; this test never rebuilds it")
        binary = Path(binary_text)
        self.assertTrue(binary.is_absolute() and binary.is_file(), "XDTO acceptance needs an absolute existing binary")
        corpus = select_profile(load_acceptance_corpus(), "agent-evaluation")
        corpus["scenarios"] = [s for s in corpus["scenarios"] if s["evaluation"] == "xdto-contract"]
        self.assertEqual(len(corpus["scenarios"]), 1)
        proofs = xdto_driver.REPO / ".session-temp/acceptance-agent"
        def driver(server, scenario):
            proof = xdto_driver.evaluate(server, scenario, proofs)
            def finish(verification):
                (proof / "verification.json").write_text(json.dumps(verification, ensure_ascii=False, indent=2))
                xdto_driver.independent_review(proof)
                print(f"accepted XDTO agent proof: {proof}", flush=True)
            return finish
        with patch.object(acceptance, "BINARY", binary):
            acceptance.run_corpus(self, corpus, scenario_driver=driver)
