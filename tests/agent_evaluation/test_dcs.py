"""Executed only by the complete large agent-evaluation suite."""
import json
import subprocess
import unittest

from tests.agent_evaluation.dcs_driver import REPO, evaluate, independent_review
from tests.ci.acceptance_controls import load_corpus
from tests.ci.acceptance_profiles import select_profile
from tests.ci.test_acceptance_scenarios import run_corpus


class DcsAgentAcceptanceTests(unittest.TestCase):
    def test_agent_uses_current_contract_without_retired_skills(self):
        subprocess.run(["cargo", "build", "--quiet", "--locked", "-p", "unica-coder", "--bin", "unica"],
                       cwd=REPO, check=True)
        corpus = select_profile(load_corpus(REPO / "tests/fixtures/acceptance/scenario-corpus.json"), "agent-evaluation")
        proofs = REPO / ".session-temp/acceptance-agent"
        def driver(server, scenario):
            proof = evaluate(server, scenario, proofs)
            def finish(verification):
                (proof / "verification.json").write_text(json.dumps(verification, ensure_ascii=False, indent=2))
                independent_review(proof)
                print(f"accepted agent proof: {proof}", flush=True)
            return finish
        run_corpus(self, corpus, scenario_driver=driver)
