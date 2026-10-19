"""Local Properties Codex acceptance against an explicitly selected binary."""
import json
import os
import unittest
from pathlib import Path
from unittest.mock import patch

from tests.agent_evaluation import props_driver
from tests.ci import test_acceptance_scenarios as acceptance
from tests.ci.acceptance_controls import load_acceptance_corpus
from tests.ci.acceptance_profiles import select_profile


class PropertiesAgentAcceptanceTests(unittest.TestCase):
    def test_agent_uses_contextual_contract_and_preserves_descriptors(self):
        binary_text = os.environ.get("UNICA_PROPERTIES_AGENT_BINARY")
        if not binary_text:
            self.fail("set UNICA_PROPERTIES_AGENT_BINARY to the immutable candidate binary; this test never rebuilds it")
        binary = Path(binary_text)
        self.assertTrue(binary.is_absolute() and binary.is_file(), "Properties acceptance needs an absolute existing candidate binary")
        corpus = select_profile(load_acceptance_corpus(), "agent-evaluation")
        corpus["scenarios"] = [s for s in corpus["scenarios"] if s["evaluation"] == "props-contract"]
        proofs = props_driver.REPO / ".session-temp/acceptance-agent"
        def driver(server, scenario):
            proof = props_driver.evaluate(server, scenario, proofs)
            def finish(verification):
                (proof / "verification.json").write_text(json.dumps(verification, ensure_ascii=False, indent=2))
                props_driver.independent_review(proof)
                print(f"accepted Properties agent proof: {proof}", flush=True)
            return finish
        with patch.object(acceptance, "BINARY", binary):
            acceptance.run_corpus(self, corpus, scenario_driver=driver)
