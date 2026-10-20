"""Pure evidence guards: no Cargo, subprocess or native scenario invocation."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "receipt_diagnostics", ROOT / "scripts/dev/receipt-causal-diagnostics/run_probe.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class ReceiptCausalDiagnosticEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.binary = (ROOT / "case-lib.exe").resolve()
        self.other = (ROOT / "case-integration.exe").resolve()
        self.binaries = {probe.LIB: self.binary, probe.INTEGRATION: self.other}

    def selection(self, *, ignored=False, path=None, duplicate=False, missing=False):
        suite = {"binary-path": str(path or self.binary), "testcases": {
            name: {"ignored": ignored, "filter-match": {"status": "matches"}}
            for name in probe.CONTROLS[:-1] if missing}}
        if not missing:
            suite["testcases"] = {name: {"ignored": ignored, "filter-match": {"status": "matches"}}
                                  for name in probe.CONTROLS}
        value = {"rust-suites": {"first": suite}}
        if duplicate:
            value["rust-suites"]["second"] = suite
        return value

    def test_exact_four_selected_from_the_lib_binary(self):
        self.assertEqual(probe.exact_selection(self.selection(), self.binaries, probe.LIB,
                                               probe.CONTROLS), probe.CONTROLS)

    def test_missing_duplicate_ignored_and_wrong_binary_are_refused(self):
        for change in ({"missing": True}, {"duplicate": True}, {"ignored": True},
                       {"path": self.other}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                probe.exact_selection(self.selection(**change), self.binaries, probe.LIB, probe.CONTROLS)

    def test_filtered_control_is_not_execution(self):
        value = self.selection()
        value["rust-suites"]["first"]["testcases"][probe.CONTROLS[0]]["filter-match"]["status"] = "mismatch"
        with self.assertRaises(ValueError):
            probe.exact_selection(value, self.binaries, probe.LIB, probe.CONTROLS)

    def junit(self, body, names, code):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "junit.xml"
            path.write_text("<testsuites><testsuite>" + body + "</testsuite></testsuites>")
            return probe.junit_cases(path, names, code)

    def test_junit_exact_pass_and_original_failure_remain_distinct(self):
        self.assertEqual(self.junit('<testcase name="a"/>', ["a"], 0), [])
        self.assertEqual(self.junit('<testcase name="a"><failure type="test failure with exit code 101"/>'
                                    '</testcase>', ["a"], 100), ["a"])

    def test_junit_zero_duplicate_skip_retry_and_exit_mismatch_are_refused(self):
        for body, code in [("", 0), ('<testcase name="a"/><testcase name="a"/>', 0),
                           ('<testcase name="a"><skipped/></testcase>', 0),
                           ('<testcase name="a"><rerunFailure/></testcase>', 0),
                           ('<testcase name="a"><failure/></testcase>', 0),
                           ('<testcase name="a"/>', 100)]:
            with self.subTest(body=body, code=code), self.assertRaises(ValueError):
                self.junit(body, ["a"], code)

    def test_partial_control_failure_retains_actual_failed_and_missing_names(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "junit.xml"
            path.write_text('<testsuite><testcase name="a"><failure type="test timeout" '
                            'message="original cause">retained detail</failure></testcase></testsuite>')
            observed = probe.junit_observation(path, ["a", "b"])
            with self.assertRaises(ValueError):
                probe.validate_junit(observed, ["a", "b"], 100)
            self.assertEqual(observed["failedTests"], ["a"])
            self.assertEqual(observed["executed"], ["a"])
            self.assertEqual(observed["missingSelected"], ["b"])
            self.assertEqual(observed["cases"][0]["causes"],
                             [{"kind": "failure", "type": "test timeout",
                               "message": "original cause", "text": "retained detail"}])

    def decision(self, cause="", code=0, text="", label="lazy32"):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "junit.xml"
            path.write_text('<testsuite><testcase name="a">' + cause + '</testcase></testsuite>')
            observed = probe.junit_observation(path, ["a"])
            probe.validate_junit(observed, ["a"], code)
            return probe.continuation_decision(observed, code, text, label)

    def test_ordinary_libtest_failure_allows_other_original_but_not_controls(self):
        cause = '<failure type="test failure with exit code 101"/>'
        for label in ("lazy32", "task4097", "controls"):
            with self.subTest(label=label):
                decision = self.decision(cause, 100, "        FAIL [   0.001s] (1/1) binary a", label)
                self.assertEqual(decision["outcome"], "ORDINARY_TEST_FAILURE")
                self.assertEqual(decision["mayContinue"], label != "controls")
        self.assertTrue(self.decision(label="controls")["mayContinue"])

    def test_typed_unsafe_and_unknown_failures_stop_without_any_raw_marker(self):
        for cause in ('<failure type="test timeout"/>',
                      '<failure type="test abort"/>',
                      '<failure type="test abort (leaked handles)"/>',
                      '<failure type="test failure with exit code 101 (leaked handles)"/>',
                      '<error type="execution failure"/>',
                      '<error type="test passed but leaked handles"/>',
                      '<error type="test exited with code 0, but leaked handles so was marked failed"/>',
                      '<failure type="test failure with exit code 1"/>',
                      '<failure type="unknown"/>', '<failure/>',
                      '<error type="test failure with exit code 101"/>',
                      '<failure type="test failure with exit code 101"/><failure type="test timeout"/>'):
            with self.subTest(cause=cause):
                self.assertFalse(self.decision(cause, 100)["mayContinue"])
        for code in (1, 4, 101, 102, 105, 110, -1):
            with self.subTest(code=code):
                self.assertFalse(self.decision('<failure type="test failure with exit code 101"/>',
                                               code)["mayContinue"])

    def test_unsafe_reporter_statuses_override_pass_and_ordinary_failure(self):
        for status in ("TIMEOUT", "TIMEOUT-PASS", "TERMINATING", "FAIL + LEAK", "PASS + LEAK",
                       "SLOW + LEAK", "SLOW+TMPASS", "LEAK", "LEAK-FAIL", "ABORT", "XFAIL",
                       "SIGKILL", "ABORT SIG 9", "TRY 2 PASS", "TRY 1 TRMNTG",
                       "FLAKY 2/3", "FLKY-FL 2/3", "UNRECOGNIZED"):
            for cause, code in (("", 0), ('<failure type="test failure with exit code 101"/>', 100)):
                with self.subTest(status=status, code=code):
                    text = f"{status:>12} [>  60.000s] (1/1) binary a\n"
                    decision = self.decision(cause, code, text)
                    self.assertFalse(decision["mayContinue"])
                    self.assertEqual(decision["unsafeStatusLines"], [text.rstrip("\n")])

    def test_assertion_prose_is_not_a_reporter_status_and_slow_alone_is_safe(self):
        text = ('TIMEOUT was the expected enum; LEAK was only an assertion label\n'
                'assertion failed: FAIL + LEAK\n'
                '        SLOW [>  60.000s] (1/1) binary a\n'
                '        FAIL [  60.001s] (1/1) binary a\n')
        decision = self.decision('<failure type="test failure with exit code 101"/>', 100, text)
        self.assertTrue(decision["mayContinue"])
        self.assertEqual(decision["unsafeStatusLines"], [])

    def test_only_inventory_known_nonselected_ignored_entries_are_allowed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "junit.xml"
            path.write_text('<testsuite><testcase name="a"/><testcase name="helper"><skipped/></testcase></testsuite>')
            self.assertEqual(probe.junit_cases(path, ["a"], 0, ["helper"]), [])
            with self.assertRaises(ValueError):
                probe.junit_cases(path, ["a"], 0)
            with self.assertRaises(ValueError):
                probe.junit_cases(path, ["a", "helper"], 0, ["helper"])

    def trace(self, label="materialized", succeeded=True, overflow=0, events=None):
        return "RECEIPT_CAUSAL " + json.dumps({"label": label, "succeeded": succeeded,
            "trace": {"capacity": 256, "overflow": overflow, "complete": overflow == 0,
                      "events": [{"operation": "actual"}] if events is None else events,
                      "firstError": None}}) + "\n"

    def task_text(self):
        return "".join("TASK4097_BRANCH " + branch + "\n" + self.trace() for branch in probe.BRANCHES)

    def test_both_task_branches_are_required_for_success(self):
        value = probe.observations(self.task_text(), "task4097", True)
        self.assertTrue(value["bothTaskBranchesObserved"])
        self.assertTrue(value["noBufferOverflow"])
        with self.assertRaises(ValueError):
            probe.observations("TASK4097_BRANCH " + probe.BRANCHES[0] + "\n" + self.trace(), "task4097", True)

    def test_first_branch_failure_retains_partial_diagnostics_without_retry(self):
        text = "TASK4097_BRANCH " + probe.BRANCHES[0] + "\n" + self.trace(succeeded=False)
        value = probe.observations(text, "task4097", False)
        self.assertFalse(value["bothTaskBranchesObserved"])
        self.assertEqual(len(value["traces"]), 1)

    def test_duplicate_unknown_and_reordered_branch_markers_are_refused(self):
        for names in ([probe.BRANCHES[1]], [probe.BRANCHES[0]] * 2, ["unknown"]):
            with self.subTest(names=names), self.assertRaises(ValueError):
                probe.observations("".join("TASK4097_BRANCH " + name + "\n" for name in names), "task4097", False)

    def test_success_without_lazy_trace_or_with_empty_trace_is_refused(self):
        for text in ("", self.trace("lazy_cancel_storm", False),
                     self.trace("wrong-label"), self.trace("lazy_cancel_storm", events=[])):
            with self.subTest(text=text), self.assertRaises(ValueError):
                probe.observations(text, "lazy32", True)

    def test_overflow_remains_incomplete_evidence_even_on_success(self):
        value = probe.observations(self.trace("lazy_cancel_storm", overflow=9), "lazy32", True)
        self.assertFalse(value["noBufferOverflow"])
        self.assertEqual(value["traces"][0]["trace"]["overflow"], 9)


if __name__ == "__main__":
    unittest.main()
