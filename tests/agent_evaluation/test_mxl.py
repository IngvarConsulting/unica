"""Локальная агентная приёмка сценария MXL."""
from scripts.ci.prepared_binary import binary_path, ensure_binary
import json
import subprocess
import unittest

from tests.agent_evaluation.mxl_driver import REPO, evaluate, independent_review
from tests.ci.acceptance_controls import load_acceptance_corpus
from tests.ci.acceptance_profiles import select_profile
from tests.ci.test_acceptance_scenarios import run_corpus


class MxlAgentAcceptanceTests(unittest.TestCase):
    def test_agent_uses_current_contract_without_retired_skills(self):
        ensure_binary(REPO)
        corpus = select_profile(load_acceptance_corpus(), "agent-evaluation")
        corpus["scenarios"] = [s for s in corpus["scenarios"] if s["evaluation"] == "mxl-contract"]
        proofs = REPO / ".session-temp/acceptance-agent"
        def driver(server, scenario):
            proof = evaluate(server, scenario, proofs)
            def finish(verification):
                (proof / "verification.json").write_text(json.dumps(verification, ensure_ascii=False, indent=2))
                independent_review(proof)
                print(f"accepted agent proof: {proof}", flush=True)
            return finish
        run_corpus(self, corpus, scenario_driver=driver)
