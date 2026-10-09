"""Counterexamples to accepting a correct class with incorrect contents."""
import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

from tests.ci.acceptance_controls import (
    FORM_AT, FORM_FILE, KNOWN_GAP, MODULE_FILE, capture_values,
    content_mismatches, load_corpus, matches_step, substitute_capture, validate_controls,
)


class ContentAssertionTests(unittest.TestCase):
    def test_successful_class_with_wrong_comment_fails(self):
        step = {"assertions": [{"pointer": "/data/props/Comment", "eq": "published"}]}
        result = {"ok": True, "data": {"props": {"Comment": "unchanged"}}}
        self.assertTrue(content_mismatches(step, result, "ok", ""),
                        "ok:true alone must not accept an unpublished comment")

    def check(self, assertion, data):
        return content_mismatches({"assertions": [assertion]}, {"ok": True, "data": data}, "ok", "")

    def test_pointer_escapes_arrays_and_root(self):
        for assertion in [{"pointer": "/data/a~1b/~0/0", "eq": "value"},
                          {"pointer": "", "eq": {"ok": True, "data": {"a/b": {"~": ["value"]}}}}]:
            self.assertEqual(self.check(assertion, {"a/b": {"~": ["value"]}}), [])

    def test_missing_is_not_null_and_not_an_inequality_success(self):
        for assertion in [{"pointer": "/data/missing", "eq": None},
                          {"pointer": "/data/missing", "notEq": "something"},
                          {"pointer": "/data/missing/child", "absent": True},
                          {"pointer": "/data/value", "absent": True}]:
            with self.subTest(assertion=assertion):
                self.assertTrue(self.check(assertion, {"value": None}))
        self.assertEqual(self.check({"pointer": "/data/missing", "absent": True}, {}), [])
        self.assertEqual(self.check({"pointer": "/data/value", "eq": None}, {"value": None}), [])

    def test_json_types_are_preserved_in_nested_equality_and_contains(self):
        for actual, expected in [(True, 1), ([True], [1]), ({"x": True}, {"x": 1})]:
            self.assertTrue(self.check({"pointer": "/data/value", "eq": expected}, {"value": actual}))
            self.assertTrue(self.check({"pointer": "/data/value", "contains": expected}, {"value": [actual]}))
        self.assertEqual(self.check({"pointer": "/data/value", "eq": 1.0}, {"value": 1}), [])

    def test_contains_is_substring_or_exact_array_element(self):
        self.assertEqual(self.check({"pointer": "/data/value", "contains": "слово"}, {"value": "два слова и слово"}), [])
        self.assertEqual(self.check({"pointer": "/data/value", "contains": {"at": "main:X"}}, {"value": [{"at": "main:X"}]}), [])
        self.assertTrue(self.check({"pointer": "/data/value", "contains": {"at": "main:X"}}, {"value": [{"at": "main:X", "extra": 1}]}))
        self.assertTrue(self.check({"pointer": "/data/value", "contains": {"x": 1}}, {"value": {"x": 1}}))

    def test_controls_reject_invalid_pointer_operator_and_extra_fields(self):
        for assertion in [{"pointer": "data/x", "eq": 1}, {"pointer": "/~2", "eq": 1},
                          {"pointer": "/x", "eq": 1, "notEq": 2}, {"pointer": "/x", "absent": False},
                          {"pointer": "/x", "eq": 1, "script": "anything"}]:
            with self.subTest(assertion=assertion), self.assertRaises(ValueError):
                validate_controls({"assertions": [assertion]})
        self.assertTrue(self.check({"pointer": "/data/array/01", "eq": 1}, {"array": [0, 1]}))
        with self.assertRaises(ValueError):
            validate_controls({"assertion": [{"pointer": "/data/value", "eq": 1}]})

    def test_exact_refusal_status_validators_and_diagnostic_remain_checked(self):
        self.assertTrue(content_mismatches({"refusal": "bad_value: exact"}, {}, "refused", "bad_value: different"))
        for field, expected in [("status", "passed"), ("validators", ["meta"]), ("diagnostic", "expected")]:
            self.assertTrue(content_mismatches({field: expected}, {"data": {}}, "ok", ""))


class FileFixtureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.workspace = Path(self.temporary.name)


class FileAndCaptureTests(FileFixtureTests):
    def test_typed_file_inequality_rejects_incompatible_operands_after_capture(self):
        (self.workspace / "module.bsl").write_bytes(b"same bytes")
        context = {"captures": {"address": "main:Module.Body"}}
        sha = {"path": "module.bsl", "kind": "sha256", "notEq": "$capture.address"}
        validate_controls({"fileAssertions": [sha]}, {"address"})
        self.assertTrue(self.check_file(sha, context))
        (self.workspace / "data.xml").write_text("<r><x>actual</x></r>")
        xml = {"path": "data.xml", "kind": "xml", "select": ".//x", "notEq": "actual"}
        with self.assertRaises(ValueError):
            validate_controls({"fileAssertions": [xml]})
        self.assertTrue(self.check_file(xml))
        for op, value in [("eq", [1]), ("contains", ["actual"])]:
            assertion = {"path": "data.xml", "kind": "xml", "select": ".//x", op: value}
            with self.subTest(op=op), self.assertRaises(ValueError):
                validate_controls({"fileAssertions": [assertion]})

    def test_duplicate_capture_keys_and_non_json_numbers_are_rejected(self):
        path = self.workspace / "corpus.json"
        for text in ['{"captures":{"hit":{"pointer":"/a"},"hit":{"pointer":"/b"}}}',
                     '{"eq":NaN}', '{"eq":1e400}', '{"eq":-1e400}']:
            path.write_text(text)
            with self.subTest(text=text), self.assertRaises(ValueError):
                load_corpus(path)
    def check_file(self, assertion, context=None):
        return content_mismatches({"fileAssertions": [assertion]}, {"ok": True}, "ok", "", self.workspace, context)

    def test_hash_before_preview_detects_a_single_changed_byte(self):
        source = self.workspace / "module.bsl"
        source.write_bytes(b"original\r\n")
        context = {"captures": capture_values({"captures": {"before": {"file": "module.bsl", "kind": "sha256"}}}, {}, self.workspace, {})}
        assertion = {"path": "module.bsl", "kind": "sha256", "eq": "$capture.before"}
        self.assertEqual(self.check_file(assertion, context), [])
        source.write_bytes(b"modified\r\n")
        self.assertTrue(self.check_file(assertion, context))

    def test_xml_preserves_order_duplicates_and_whitespace(self):
        (self.workspace / "data.xml").write_text('<r xmlns="urn:test"><x> first </x><x>second</x><x>second</x></r>')
        assertion = {"path": "data.xml", "kind": "xml", "select": ".//{urn:test}x", "eq": [" first ", "second", "second"]}
        self.assertEqual(self.check_file(assertion), [])
        for value in [["first", "second", "second"], [" first ", "second"], ["second", " first ", "second"]]:
            self.assertTrue(self.check_file({**assertion, "eq": value}))

    def test_file_controls_refuse_missing_paths_traversal_and_symlinks(self):
        (self.workspace / "real").write_bytes(b"data")
        (self.workspace / "link").symlink_to(self.workspace / "real")
        for path in ["missing", "../outside", "/absolute", "C:/outside", "link"]:
            with self.subTest(path=path):
                self.assertTrue(self.check_file({"path": path, "kind": "sha256", "eq": hashlib.sha256(b"data").hexdigest()}))

    def test_xml_rejects_malformed_documents_selectors_and_entities(self):
        assertion = {"path": "data.xml", "kind": "xml", "select": ".//x", "eq": []}
        for raw in [b"<r>", b'<!DOCTYPE r [<!ENTITY x "expanded">]><r>&x;</r>',
                    '<!DOCTYPE r [<!ENTITY x "expanded">]><r>&x;</r>'.encode("utf-16")]:
            (self.workspace / "data.xml").write_bytes(raw)
            self.assertTrue(self.check_file(assertion))
        with self.assertRaises(ValueError):
            validate_controls({"fileAssertions": [{**assertion, "select": ".//["}]})

    def test_capture_extracts_address_and_missing_or_overwrite_fails(self):
        step = {"captures": {"hitAt": {"pointer": "/data/items/0/at"}}}
        context = {"captures": capture_values(step, {"data": {"items": [{"at": "main:Module.Body"}]}}, self.workspace, {})}
        self.assertEqual(substitute_capture({"at": "$capture.hitAt"}, context), {"at": "main:Module.Body"})
        with self.assertRaises(ValueError):
            capture_values(step, {"data": {"items": [{}]}}, self.workspace, {})
        with self.assertRaises(ValueError):
            capture_values(step, {}, self.workspace, context)
        self.assertEqual(context["captures"], {"hitAt": "main:Module.Body"})
        with self.assertRaises(ValueError):
            substitute_capture("$capture.unset", context)

    def test_capture_schema_rejects_future_reserved_duplicate_and_mixed_sources(self):
        for step in [{"args": {"at": "$capture.future"}},
                     {"captures": {"rev": {"pointer": "/rev"}}},
                     {"captures": {"old": {"pointer": "/data"}}},
                     {"captures": {"value": {"pointer": "/data", "file": "x", "kind": "sha256"}}}]:
            with self.subTest(step=step), self.assertRaises(ValueError):
                validate_controls(step, {"old"})


class KnownGapTests(FileFixtureTests):
    def step(self):
        return {"tool": "unica.check", "args": {"at": FORM_AT}, "expect": ["gap"],
                "gap": "#791: событие имеет три параметра вместо пяти", "status": "passed", "validators": ["form"],
                "knownGap": {"name": KNOWN_GAP, "observedClass": "ok", "issue": 791}}

    def witness(self, correct=False):
        form = self.workspace / FORM_FILE
        form.parent.mkdir(parents=True)
        form.write_text('<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><InputField name="ПолеНаименование"><Events><Event name="ChoiceProcessing">ПолеНаименованиеОбработкаВыбора</Event></Events></InputField></Form>')
        module = self.workspace / MODULE_FILE
        module.parent.mkdir()
        params = "Элемент, ВыбранноеЗначение, СтандартнаяОбработка"
        if correct:
            params = "Элемент, ВыбранноеЗначение, ДополнительныеДанные, ВыборДобавлением, СтандартнаяОбработка"
        module.write_text(f"&НаКлиенте\nПроцедура ПолеНаименованиеОбработкаВыбора({params})\nКонецПроцедуры\n")

    def test_only_the_witnessed_incorrect_verdict_matches(self):
        self.witness()
        step = self.step()
        validate_controls(step)
        result = {"ok": True, "data": {"status": "passed", "validators": ["form"]}}
        self.assertTrue(matches_step(step, "ok"))
        self.assertEqual(content_mismatches(step, result, "ok", "", self.workspace), [])
        for change in [{"ok": "false"}, {"data": {"status": "failed", "validators": ["form"]}},
                       {"data": {"status": "passed", "validators": ["meta"]}}]:
            self.assertTrue(content_mismatches(step, {**result, **change}, "ok", "", self.workspace))
        self.assertFalse(matches_step(step, "failed"))
        self.assertFalse(matches_step(step, "provider"))

    def test_healthy_or_missing_fixture_cannot_be_a_known_gap(self):
        result = {"ok": True, "data": {"status": "passed", "validators": ["form"]}}
        self.assertTrue(content_mismatches(self.step(), result, "ok", "", self.workspace))
        self.witness(correct=True)
        self.assertTrue(content_mismatches(self.step(), result, "ok", "", self.workspace))

    def test_unregistered_issue_target_or_observation_is_rejected(self):
        for key, value in [("issue", 790), ("issue", 791.0), ("name", "any-ok"), ("observedClass", "provider")]:
            step = self.step()
            step["knownGap"][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                validate_controls(step)
        step = self.step()
        step["args"] = {"at": "main:Catalog.Другой.Form.Форма"}
        with self.assertRaises(ValueError):
            validate_controls(step)


class GapPresentationTests(unittest.TestCase):
    def test_site_and_registry_preserve_gap_evidence_and_count_steps(self):
        root = Path(__file__).resolve().parents[2]
        modules = []
        for name in ["scenario-status", "render-acceptance-registry"]:
            spec = importlib.util.spec_from_file_location(name, root / "scripts/ci" / f"{name}.py")
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            modules.append(module)
        status, registry = modules
        known = {"tool": "unica.check", "args": {"at": FORM_AT}, "expect": ["gap"],
                 "gap": "known incorrect verdict", "status": "passed", "validators": ["form"],
                 "knownGap": {"name": KNOWN_GAP, "observedClass": "ok", "issue": 791}}
        corpus = {"scenarios": [{"id": "S001", "area": "Forms", "task": "Witness the defect", "wire": [known, {"tool": "unica.view", "args": {}, "expect": ["gap"], "gap": "other defect"}]}]}
        view = status.build(corpus)
        detailed = json.loads(view["scenarios_json"])
        self.assertEqual(detailed[0]["wire"][0]["knownGap"], known["knownGap"])
        self.assertEqual(detailed[0]["wire"][0]["gap"], known["gap"])
        self.assertEqual(view["gap_steps_total"], "2")
        self.assertEqual(view["gap_scenarios_total"], "1")
        rendered = registry.render(corpus, [], set(), [])
        self.assertIn("known incorrect verdict", rendered)
        self.assertIn("(#791)", rendered)
        self.assertIn("**2** в **1 сценариях**", rendered)


if __name__ == "__main__":
    unittest.main()
