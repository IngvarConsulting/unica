"""Local Codex acceptance; needs installed 8.3.27 help and an immutable binary."""
import json
import os
import unittest
from pathlib import Path
from unittest.mock import patch

from tests.agent_evaluation import event_data_driver as driver
from tests.ci import test_acceptance_scenarios as acceptance
from tests.ci.acceptance_controls import load_acceptance_corpus
from tests.ci.acceptance_profiles import select_profile


class EventDataAgentAcceptanceTests(unittest.TestCase):
    def test_agent_verifies_data_visibility_without_mutation(self):
        binary = Path(os.environ.get("UNICA_EVENT_DATA_AGENT_BINARY", ""))
        self.assertTrue(binary.is_absolute() and binary.is_file(),
                        "set UNICA_EVENT_DATA_AGENT_BINARY to an immutable candidate binary")
        corpus = select_profile(load_acceptance_corpus(), "agent-evaluation")
        corpus["scenarios"] = [s for s in corpus["scenarios"] if s["evaluation"] == "event-data-visibility"]
        self.assertEqual(len(corpus["scenarios"]), 1)
        def evaluate(server, scenario):
            proof = driver.evaluate(server, scenario, driver.REPO / ".session-temp/acceptance-agent")
            def finish(verification):
                (proof / "verification.json").write_text(json.dumps(verification, ensure_ascii=False, indent=2))
                driver.independent_review(proof)
                print(f"accepted event data visibility proof: {proof}", flush=True)
            return finish
        with patch.object(acceptance, "BINARY", binary):
            acceptance.run_corpus(self, corpus, scenario_driver=evaluate)
