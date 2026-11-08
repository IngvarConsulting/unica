"""The acceptance corpus executed against the built product through delivery and fault profiles.

Split out of `test_acceptance_scenarios.py`, whose helpers it uses.
"""

# The helpers stay in `test_acceptance_scenarios`: tests elsewhere patch them
# there by module path, so they keep one home.
from tests.ci.test_acceptance_scenarios import *  # noqa: F401,F403
# Their own tests run in their own module, not again here.
del AcceptanceCorpusRunTests, AcceptanceCorpusScheduleTests  # noqa: F821

# Размер тестов модуля (scripts/ci/python_sizes.py).
PR_SKIP = "малым тестам нужен продукт"
# Приёмочный корпус: собирает продукт и гоняет сценарии через MCP.
MEDIUM = ("AcceptanceDeliveryCorpusRunTests", "AcceptanceFaultCorpusRunTests")


class AcceptanceDeliveryCorpusRunTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        ensure_binary(REPO_ROOT)
        cls.corpus = load_acceptance_corpus()
        for scenario in cls.corpus["scenarios"]:
            declared = set()
            for step in scenario["wire"]:
                validate_controls(step, declared)

    def test_analyzer_delivery_wires_have_terminal_content(self):
        """
        rule: INV.APP.DIAGNOSTIC-SUPPRESSION-EVIDENCE
        rule: INV.RUNTIME.SERVICE-MUTATION-INVALIDATION
        """
        import sys
        corpus = select_profile(self.corpus, "delivery")
        with tempfile.TemporaryDirectory(prefix="unica-analyzer-delivery-") as raw:
            plugin = Path(raw).resolve() / "plugin"
            subprocess.run([sys.executable, "-m", "tests.ci.acceptance_profiles", "--stage-analyzer", str(plugin)],
                           cwd=REPO_ROOT, timeout=90, check=True)
            from tests.ci.acceptance_diagnostics import prepare_diagnostic_case
            with ExitStack() as fixtures:
                def prepare(server, scenario):
                    return prepare_diagnostic_case(server, scenario, plugin=plugin, fixtures=fixtures)
                run_corpus(self, corpus, plugin, scenario_driver=prepare,
                           fixture_cleanup=fixtures.close)
    def test_pinned_resident_reports_real_baseline_author_and_scope_facts(self):
        """rule: INV.APP.DIAGNOSTIC-SUPPRESSION-EVIDENCE"""
        from tests.ci.acceptance_profiles import stage_analyzer, host_target
        from tests.ci.acceptance_diagnostics import resident_filter_evidence
        import platform
        with tempfile.TemporaryDirectory(prefix="unica-resident-filter-") as raw:
            root=Path(raw).resolve()
            plugin=stage_analyzer(root / "plugin")
            lock=json.loads((plugin / "third-party/tools.lock.json").read_text())
            target=host_target(lock,platform.system(),platform.machine())
            binary=plugin / "bin" / target / ("bsl-analyzer.exe" if os.name=="nt" else "bsl-analyzer")
            workspace=root / "workspace"
            shutil.copytree(REPO_ROOT / DIAGNOSTICS_WORKSPACE,workspace)
            evidence=resident_filter_evidence(binary,workspace,root)
            self.assertEqual(len(evidence["plain"]["result"]["findings"]),3)
            self.assertEqual(evidence["authors"]["result"]["findings_ignored_by_author"],3)
            self.assertEqual(evidence["authors"]["result"]["findings"],[])
            self.assertIs(evidence["scope"]["result"]["out_of_scope"],True)
            self.assertEqual(evidence["both"]["result"]["baseline"]["known"],1)
            self.assertEqual(evidence["both"]["result"]["baseline"]["new"],2)
            self.assertEqual(evidence["both"]["result"]["findings_ignored_by_author"],2)
            self.assertEqual(evidence["both"]["result"]["findings"],[])


class AcceptanceFaultCorpusRunTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        ensure_binary(REPO_ROOT)
        cls.corpus = load_acceptance_corpus()

    def test_controlled_jsonl_faults_preserve_public_reason_without_provider_input(self):
        """rule: INV.APP.DIAGNOSTIC-PROVIDERS"""
        from tests.ci.acceptance_faults import stage_fault_producer, fault_driver
        with tempfile.TemporaryDirectory(prefix="unica-jsonl-fault-") as raw:
            plugin = Path(raw).resolve() / "plugin"
            manifest = stage_fault_producer(plugin)
            self.assertFalse(manifest["testFixture"]["publishedArtifact"])
            self.assertEqual(manifest["tools"][0]["version"], "0.0.0-test-fixture")
            records = []
            run_corpus(self, fault_cases(self.corpus, "analyzer-jsonl-fault"), plugin,
                       scenario_driver=fault_driver(plugin, records))
            self.assertEqual(sum(record["tool"] == "unica.check" for record in records), 6)

    def test_call_graph_branch_names_a_failed_analyzer_delivery(self):
        from tests.ci.acceptance_faults import stage_missing_analyzer, delivery_failure_driver
        with tempfile.TemporaryDirectory(prefix="unica-delivery-failure-") as raw:
            plugin = Path(raw).resolve() / "plugin"
            manifest = stage_missing_analyzer(plugin)
            self.assertFalse(manifest["testFixture"]["publishedArtifact"])
            cases = fault_cases(self.corpus, "analyzer-delivery-failure")
            self.assertEqual([scenario["id"] for scenario in cases["scenarios"]], ["S396"])
            run_corpus(self, cases, plugin, scenario_driver=delivery_failure_driver(plugin))


if __name__ == "__main__":
    unittest.main()
