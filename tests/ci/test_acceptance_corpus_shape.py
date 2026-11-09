"""The shape of the acceptance corpus: scenarios, profiles and documented gaps.

Split out of `test_acceptance_scenarios.py`, whose helpers it uses.
"""

# The helpers stay in `test_acceptance_scenarios`: tests elsewhere patch them
# there by module path, so they keep one home.
from tests.ci.test_acceptance_scenarios import *  # noqa: F401,F403
# Their own tests run in their own module, not again here.
del AcceptanceCorpusRunTests, AcceptanceCorpusScheduleTests  # noqa: F821

# Размер тестов модуля (scripts/ci/python_sizes.py).
PR_SKIP = "малым тестам нужен продукт"


class AcceptanceCorpusShapeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.corpus = load_acceptance_corpus()

    def test_the_packed_json_guard_sees_nested_strings_and_has_view_steps_to_check(self) -> None:
        answer = {"data": {"items": [{"props": {
            "type": '{"variants":[]}',
            "bounds": ' [1, 2]',
            "text": "{not json",
            "nested": {"kind": "reference"},
        }}]}}
        self.assertEqual(
            [finding.split(" packs")[0] for finding in packed_json_strings(answer)],
            ["/data/items/0/props/type", "/data/items/0/props/bounds"],
        )
        views = [step for scenario in self.corpus["scenarios"] for step in scenario["wire"]
                 if step["tool"] == "unica.view"]
        self.assertGreater(len(views), 0, "the guard runs on `unica.view` steps")

    def test_corpus_ids_are_stable_and_the_manifest_freezes_every_step(self) -> None:
        storage = corpus_storage()
        scenarios = self.corpus["scenarios"]
        identifiers = [scenario["id"] for scenario in scenarios]
        self.assertEqual(len(identifiers), len(set(identifiers)), "scenario ids are unique")
        # The historical block keeps its numbers and is closed: links to S###
        # still find their scenario, and nothing takes the next number.
        legacy = [identifier for identifier in identifiers if storage.LEGACY_ID.fullmatch(identifier)]
        self.assertEqual(legacy, [f"S{index:03d}" for index in range(1, storage.LEGACY_LAST + 1)])
        for identifier in identifiers[len(legacy):]:
            storage.stable_order(identifier)
        # A dropped scenario or step is caught per scenario, not by a total
        # every addition would have to edit.
        self.assertEqual(
            storage.manifest_problems(self.corpus, storage.read_manifest()), [],
            "the corpus differs from tests/fixtures/acceptance/scenario-manifest/",
        )

    def test_all_scenarios_have_an_executable_profile(self) -> None:
        source = {s["id"] for s in select_profile(self.corpus, "source")["scenarios"]}
        delivery = {s["id"] for s in select_profile(self.corpus, "delivery")["scenarios"]}
        agent = {s["id"] for s in select_profile(self.corpus, "agent-evaluation")["scenarios"]}
        fault = {s["id"] for s in select_profile(self.corpus, "fault-injection")["scenarios"]}
        self.assertFalse(fault & (source | delivery | agent))
        self.assertFalse(source & delivery or source & agent or delivery & agent)
        runtime = {s["id"] for s in select_profile(self.corpus, "runtime-fault")["scenarios"]}
        self.assertFalse(runtime & (source | delivery | agent | fault))
        self.assertEqual(source | delivery | agent | fault | runtime, {s["id"] for s in self.corpus["scenarios"]})
        # The closed S### block keeps its profiles; a per-file scenario
        # declares its own and is admitted by select_profile, so two added
        # scenarios never edit these sets.
        legacy = corpus_storage().LEGACY_ID.fullmatch
        self.assertEqual({i for i in fault if legacy(i)}, {"S344", "S345", "S346", "S389", "S390", "S396", "S398"})
        self.assertEqual({i for i in agent if legacy(i)}, {"S328", "S335"})
        self.assertEqual({i for i in delivery if legacy(i)}, {'S324', 'S339', 'S340', 'S341', 'S342', 'S343', 'S359'})

    def test_every_step_freezes_known_classes_and_documents_gaps(self) -> None:
        for scenario in self.corpus["scenarios"]:
            declared = set()
            workspace = scenario.get("workspace", self.corpus["workspace"])
            self.assertIn(
                workspace,
                SOURCE_WORKSPACES | {SYMBOL_WORKSPACE, DIAGNOSTICS_WORKSPACE, FORM_RUNTIME_WORKSPACE, PULL_VALIDATION_WORKSPACE},
                f"{scenario['id']}: a scenario runs on one of the registered fixture workspaces",
            )
            for index, step in enumerate(scenario["wire"]):
                with self.subTest(scenario=scenario["id"], step=index):
                    validate_controls(step, declared)
                    self.assertTrue(step["tool"].startswith("unica."))
                    expected = step["expect"]
                    self.assertTrue(expected, "every step freezes an expectation")
                    self.assertTrue(set(expected) <= EXPECT_CLASSES, expected)
                    if "form" in step:
                        self.assertIs(step["form"], True, "a form step is marked `form: true`")
                    if "gap" in expected:
                        self.assertTrue(
                            step.get("gap"),
                            "a documented gap names its reason",
                        )
                    if "refused" in expected:
                        refusal = step.get("refusal") or ""
                        self.assertTrue(refusal, "an intended refusal freezes its message")
                        code = refusal.split(":", 1)[0]
                        self.assertIn(
                            code,
                            REFUSAL_CODES,
                            "a refusal is frozen as `code: message` with a code from the closed set",
                        )
                    if step["tool"] == "unica.check" and step["args"].get("at") and expected == ["ok"]:
                        # A check over a node freezes the verdict it expects and
                        # the validators the node kind owns; the caller never
                        # names a validator, so the corpus is where the
                        # kind-to-validator table is witnessed.
                        self.assertIn(
                            step.get("status"),
                            {"passed", "failed", "readable"},
                            "a check over a node freezes the verdict it expects",
                        )
                        self.assertIsInstance(
                            step.get("validators"),
                            list,
                            "a check over a node freezes the validators its kind owns",
                        )

    def test_refusal_classes_come_from_the_product_dictionary(self) -> None:
        def classify_diagnostic(code):
            response = {"result": {"structuredContent": {
                "ok": False, "diagnostics": [{"code": code, "message": "m"}]}}}
            return classify(response, {})

        # The harness reads the closed dictionary instead of keeping a copy:
        # every variant is found, and a legitimate caller-side refusal such
        # as `view`'s foreign cursor is a `refused`, not a gap.
        dictionary = refusal_dictionary()
        declared = re.search(
            r"pub const ALL: \[Self; (\d+)\]",
            REFUSAL_DICTIONARY.read_text(encoding="utf-8"),
        )
        self.assertIsNotNone(declared)
        assert declared is not None
        self.assertEqual(len(dictionary), int(declared.group(1)))
        self.assertLessEqual(REFUSAL_CODES, set(dictionary))
        self.assertLessEqual(
            {"bad_value", "not_found", "invalid_state", "invalid_source",
             "stale_revision", "invalid_cursor"},
            REFUSAL_CODES,
        )
        self.assertEqual(dictionary["invalid_cursor"], "FixCall")
        self.assertEqual(dictionary["invalid_state"], "NeedsHuman")
        # Environment failures are not refusals of the request.
        for failure in ("rollback_incomplete", "task_backend_failed"):
            self.assertIn(failure, dictionary)
            self.assertNotIn(failure, REFUSAL_CODES)
        for code in REFUSAL_CODES:
            self.assertTrue(
                dictionary[code] in CALLER_OUTCOMES or code in NEEDS_HUMAN_REFUSALS,
                code,
            )
            # `unsupported` and `provider` own their codes before `refused`.
            self.assertFalse(code.startswith("unsupported_"), code)
        self.assertIn("target_kind_unsupported", dictionary)
        self.assertNotIn("target_kind_unsupported", REFUSAL_CODES)
        self.assertEqual(
            classify_diagnostic("target_kind_unsupported")[0], "unsupported"
        )
        # A failed product postcondition is a defect, never an intended refusal.
        self.assertEqual(dictionary["postcondition_failed"], "FixSource")
        self.assertNotIn("postcondition_failed", REFUSAL_CODES)
        self.assertEqual(
            classify_diagnostic("postcondition_failed")[0], "gap-candidate"
        )
        self.assertNotIn("provider_unavailable", REFUSAL_CODES)

    def test_gaps_stay_a_bounded_exception_not_a_habit(self) -> None:
        gap_steps = [
            (scenario["id"], step)
            for scenario in self.corpus["scenarios"]
            for step in scenario["wire"]
            if "gap" in step["expect"]
        ]
        # Every documented gap names a reproduced surface defect in its text.
        # The rendered registry lists them; the ceiling from the fixture README
        # (two) is a ratchet against silent growth, not a target.
        self.assertLessEqual(
            len(gap_steps),
            2,
            "documented gaps grew: either fix the surface or re-approve the corpus",
        )


if __name__ == "__main__":
    unittest.main()
