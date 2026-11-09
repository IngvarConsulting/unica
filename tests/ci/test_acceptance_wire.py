"""Wire controls of acceptance steps: saved plans and external link fixtures.

Split out of `test_acceptance_scenarios.py`, whose helpers it uses.
"""

# The helpers stay in `test_acceptance_scenarios`: tests elsewhere patch them
# there by module path, so they keep one home.
from tests.ci.test_acceptance_scenarios import *  # noqa: F401,F403
# Their own tests run in their own module, not again here.
del AcceptanceCorpusRunTests, AcceptanceCorpusScheduleTests  # noqa: F821

# Размер тестов модуля (scripts/ci/python_sizes.py).
PR_SKIP = "малым тестам нужен продукт"


class ExternalLinkFixtureTests(unittest.TestCase):
    def test_fixed_link_is_real_and_the_neighbor_bytes_are_preserved(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw) / "workspace"
            shutil.copytree(REPO_ROOT / "tests/fixtures/acceptance/workspace-external-linked", workspace)
            target = workspace / "epf/Other/Forms/Main/Ext/Form.xml"
            original = target.read_bytes()
            materialize_external_form_link(workspace)
            payload = workspace / "epf/Import/Forms/Main/Ext/Form.xml"
            self.assertTrue(payload.is_symlink())
            self.assertEqual(os.readlink(payload), "../../../../Other/Forms/Main/Ext/Form.xml")
            self.assertEqual(payload.resolve(strict=True), target.resolve(strict=True))
            self.assertEqual(target.read_bytes(), original)
            with self.assertRaises(ValueError):
                materialize_external_form_link(workspace)


    def test_link_refusal_cannot_pass_as_wrong_descriptor_identity(self):
        corpus = load_acceptance_corpus()
        case = next(s for s in corpus["scenarios"] if s.get("name") == "external-linked-payload-refusal")
        link = {"assertions": case["wire"][0]["assertions"]}
        foreign = case["wire"][1]["assertions"][1]["eq"]
        wrong_cause = {"ok": False, "diagnostics": foreign}
        mismatches = content_mismatches(link, wrong_cause, "provider", "")
        self.assertEqual(len(mismatches), 1)
        self.assertIn("notEq", mismatches[0])


class SavedPlanWireTests(unittest.TestCase):
    def test_capture_values_are_opaque_even_when_they_look_like_template_tokens(self):
        context = {"captures": {"text": "$executionToken", "object": {"at": "$rev"}},
                   "executionToken": "saved-plan", "rev": "current-revision"}
        self.assertEqual(substitute({"text": "$capture.text", "object": "$capture.object"}, context),
                         {"text": "$executionToken", "object": {"at": "$rev"}})

    def test_absolute_path_query_stays_inside_the_scenario_workspace(self):
        context = {"workspace": Path("/scenario")}
        self.assertEqual(substitute({"path": "$workspace/src/Form.xml"}, context),
                         {"path": str(Path("/scenario/src/Form.xml"))})
        for path in ["$workspace/../outside.xml", "$workspace/C:/outside.xml", "$workspace//outside.xml"]:
            with self.subTest(path=path), self.assertRaises(ValueError):
                substitute(path, context)
        with self.assertRaises(ValueError):
            substitute("$workspace/src/Form.xml", {})
        self.assertEqual(substitute("$capture.literal", {"captures": {"literal": "$workspace/src/Form.xml"}}),
                         "$workspace/src/Form.xml")

    def test_execute_uses_the_preview_token_and_reads_do_not_replace_it(self):
        context = {}
        plan = {"result": {"structuredContent": {
            "ok": True, "rev": "source-revision", "data": {"executionToken": "saved-plan-token"}
        }}}
        self.assertEqual(classify(plan, context)[0], "ok")
        classify({"result": {"structuredContent": {"ok": True, "rev": "read-revision"}}}, context)
        self.assertEqual(substitute({"executionToken": "$executionToken"}, context),
                         {"executionToken": "saved-plan-token"})
        self.assertEqual(context["rev"], "read-revision")

    def test_only_token_execution_requires_an_isolated_mutable_workspace(self):
        self.assertFalse(scenario_publishes({"wire": [{"tool": "unica.apply", "args": {
            "at": "main:Configuration", "ops": []
        }}]}))
        self.assertTrue(scenario_publishes({"wire": [{"tool": "unica.apply", "args": {
            "executionToken": "$executionToken"
        }}]}))


if __name__ == "__main__":
    unittest.main()
