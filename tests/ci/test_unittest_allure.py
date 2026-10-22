"""Allure preserves unittest fixture failures without interrupting later tests."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import itertools
import json
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock


RUNNER = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "run-unittest.py"


def load_runner():
    spec = importlib.util.spec_from_file_location("unittest_allure_runner", RUNNER)
    module = importlib.util.module_from_spec(spec)
    with mock.patch.object(sys, "path", list(sys.path)):
        spec.loader.exec_module(module)
    return module


class UnittestAllureTests(unittest.TestCase):
    def setUp(self):
        self.runner = load_runner()
        self.directory = tempfile.TemporaryDirectory(prefix="unittest-allure-")
        self.addCleanup(self.directory.cleanup)
        self.out = Path(self.directory.name) / "results"
        self.runner.AllureResult.out = self.out
        self.runner.AllureResult.runner_name = "fixture-runner"
        self.runner.AllureResult.profile = "queue"
        self.runner.AllureResult.suite_name = "tests/fixture"

    def records(self):
        return {
            entry["fullName"]: entry
            for path in self.out.glob("*-result.json")
            for entry in [json.loads(path.read_text(encoding="utf-8"))]
        }

    def run_fixture(self, hook, exception):
        self.out = Path(self.directory.name) / f"results-{hook}-{exception.__name__}"
        self.runner.AllureResult.out = self.out
        source = types.ModuleType("allure_fixture_source")
        following = types.ModuleType("allure_fixture_following")
        events = []
        marker = f"original {hook} failure"

        def fail_fixture(_class=None):
            raise exception(marker)

        def case(name, module):
            def test_body(_self):
                events.append(name)

            result = type(name, (unittest.TestCase,), {
                "__module__": module.__name__, "test_body": test_body,
            })
            setattr(module, name, result)
            return result

        affected = case("Affected", source)
        case("FollowingClass", source)
        case("FollowingModule", following)
        if hook.endswith("Class"):
            setattr(affected, hook, classmethod(fail_fixture))
            owner = "allure_fixture_source.Affected"
        else:
            setattr(source, hook, fail_fixture)
            owner = "allure_fixture_source"
        self.runner.AllureResult.sizes = self.runner.Sizes([owner])
        suite = unittest.TestSuite(
            unittest.TestLoader().loadTestsFromModule(module)
            for module in (source, following)
        )
        output = io.StringIO()
        with mock.patch.dict(sys.modules, {
            source.__name__: source, following.__name__: following,
        }), mock.patch.object(self.runner.allure_results, "now_ms", side_effect=itertools.count(100)):
            result = unittest.TextTestRunner(
                stream=output, resultclass=self.runner.AllureResult,
            ).run(suite)
        return result, self.records(), events, owner, marker, output.getvalue()

    def check_fixture(self, hook, exception):
        result, records, events, owner, marker, output = self.run_fixture(hook, exception)
        fixture = records[f"{hook} ({owner})"]
        skipped = exception is unittest.SkipTest
        self.assertEqual(fixture["status"], "skipped" if skipped else "broken")
        self.assertIn(marker, fixture["statusDetails"]["message"])
        if not skipped:
            trace = fixture["statusDetails"]["trace"]
            self.assertIn("RuntimeError: " + marker, trace)
            self.assertIn("in fail_fixture", trace)
            self.assertIn(marker, output)
        self.assertNotIn("_testMethodName", output)
        self.assertNotIn("_testMethodDoc", output)
        self.assertEqual(fixture["start"], fixture["stop"])
        labels = {label["name"]: label["value"] for label in fixture["labels"]}
        self.assertEqual(labels["subSuite"], owner)
        self.assertEqual(labels["size"], "medium")
        self.assertEqual(labels["suite"], "tests/fixture")
        self.assertEqual(labels["profile"], "queue")
        self.assertEqual(fixture["parameters"], [{"name": "runner", "value": "fixture-runner"}])
        self.assertEqual(len(result.errors), 0 if skipped else 1)
        self.assertEqual(len(result.skipped), 1 if skipped else 0)
        self.assertEqual(len(result.failures), 0)
        self.assertEqual(result.wasSuccessful(), skipped)
        expected = ["Affected", "FollowingClass", "FollowingModule"]
        if hook == "setUpClass":
            expected.remove("Affected")
        elif hook == "setUpModule":
            expected = ["FollowingModule"]
        self.assertEqual(events, expected)
        self.assertEqual(result.testsRun, len(expected))
        self.assertEqual(len(records), len(expected) + 1)
        self.assertEqual(records["allure_fixture_following.FollowingModule.test_body"]["status"], "passed")

    def test_class_and_module_fixture_errors_keep_traceback_and_continue(self):
        for hook in ("setUpClass", "tearDownClass", "setUpModule", "tearDownModule"):
            with self.subTest(hook=hook):
                self.check_fixture(hook, RuntimeError)

    def test_class_and_module_fixture_skips_preserve_unittest_counts(self):
        for hook in ("setUpClass", "tearDownClass", "setUpModule", "tearDownModule"):
            with self.subTest(hook=hook):
                self.check_fixture(hook, unittest.SkipTest)

    def test_ordinary_results_keep_names_statuses_and_labels(self):
        class Ordinary(unittest.TestCase):
            def test_passed(self):
                """Existing caption.

                Longer description is not the caption.
                """

            def test_failed(self):
                self.fail("ordinary assertion")

            def test_broken(self):
                raise RuntimeError("ordinary error")

            @unittest.skip("ordinary skip")
            def test_skipped(self):
                pass

            @unittest.expectedFailure
            def test_expected(self):
                self.fail("expected")

            @unittest.expectedFailure
            def test_unexpected(self):
                pass

        prefix = f"{Ordinary.__module__}.{Ordinary.__qualname__}"
        self.runner.AllureResult.sizes = self.runner.Sizes([prefix + ".test_passed"])
        with mock.patch.object(self.runner.allure_results, "now_ms", side_effect=itertools.count(100)):
            result = unittest.TextTestRunner(
                stream=io.StringIO(), resultclass=self.runner.AllureResult,
            ).run(unittest.TestLoader().loadTestsFromTestCase(Ordinary))
        self.assertEqual(result.testsRun, 6)
        records = self.records()
        expected = {
            "test_passed": "passed", "test_failed": "failed", "test_broken": "broken",
            "test_skipped": "skipped", "test_expected": "passed", "test_unexpected": "failed",
        }
        self.assertEqual(set(records), {f"{prefix}.{name}" for name in expected})
        for method, status in expected.items():
            entry = records[f"{prefix}.{method}"]
            self.assertLess(entry["start"], entry["stop"])
            self.assertEqual(entry["status"], status)
            self.assertEqual(entry["name"], "Existing caption." if method == "test_passed" else method)
            self.assertEqual({label["name"]: label["value"] for label in entry["labels"]}, {
                "language": "python", "framework": "unittest", "parentSuite": "python",
                "suite": "tests/fixture", "subSuite": Ordinary.__qualname__, "profile": "queue",
                "size": "medium" if method == "test_passed" else "small",
                "host": "fixture-runner", "tag": "queue",
            })

    def test_main_returns_nonzero_and_records_later_module_after_setup_error(self):
        root = Path(self.directory.name) / "suite"
        root.mkdir()
        (root / "test_allure_init_failure.py").write_text(
            "import unittest\n"
            "class Broken(unittest.TestCase):\n"
            "    @classmethod\n"
            "    def setUpClass(cls):\n"
            "        raise RuntimeError('original initialization traceback')\n"
            "    def test_never_runs(self):\n"
            "        self.fail('setup did not run')\n",
            encoding="utf-8",
        )
        (root / "test_allure_z_following.py").write_text(
            "import unittest\n"
            "class Later(unittest.TestCase):\n"
            "    def test_runs(self):\n"
            "        pass\n",
            encoding="utf-8",
        )
        output = io.StringIO()
        with mock.patch.object(sys, "path", list(sys.path)), mock.patch.dict(sys.modules), \
                mock.patch.object(unittest, "defaultTestLoader", unittest.TestLoader()), \
                contextlib.redirect_stderr(output):
            code = self.runner.main(["-s", str(root), "--results", str(self.out)])
        self.assertEqual(code, 1)
        self.assertIn("RuntimeError: original initialization traceback", output.getvalue())
        records = self.records()
        self.assertEqual(records["setUpClass (test_allure_init_failure.Broken)"]["status"], "broken")
        self.assertEqual(records["test_allure_z_following.Later.test_runs"]["status"], "passed")
        self.assertEqual(len(records), 2)


if __name__ == "__main__":
    unittest.main()
