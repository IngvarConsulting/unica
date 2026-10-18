"""Диагностика реального дочернего процесса при построении плана Rust."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "allure_results.py"
HARNESS = """
import importlib.util
import json
import subprocess
import sys
from pathlib import Path
spec = importlib.util.spec_from_file_location('allure_results', sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
try:
    entries = module.nextest_list(Path(sys.argv[2]), [sys.executable, '-c', sys.argv[3], sys.argv[4]])
except subprocess.CalledProcessError as error:
    raise SystemExit(error.returncode)
print(json.dumps(entries))
"""
CHILD = """
import json
import sys
sys.stderr.write('native-nextest-diagnostic-sentinel\\n')
if sys.argv[1] == 'failure':
    raise SystemExit(19)
print(json.dumps({'rust-suites': {'sample': {
    'binary-id': 'unica-coder',
    'testcases': {
        'case::selected': {'filter-match': {'status': 'matches'}},
        'case::excluded': {'filter-match': {'status': 'mismatch'}},
    },
}}}))
"""


class NextestPlanOutputTests(unittest.TestCase):
    def invoke(self, mode: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory(prefix="nextest-plan-output-") as directory:
            return subprocess.run(
                [sys.executable, "-c", HARNESS, str(MODULE_PATH), directory, CHILD, mode],
                capture_output=True, text=True, check=False, timeout=15,
            )

    def test_failed_plan_exposes_native_diagnostic_and_preserves_failure(self) -> None:
        result = self.invoke("failure")
        self.assertEqual(result.returncode, 19, result.stderr)
        self.assertIn("native-nextest-diagnostic-sentinel", result.stderr)
        self.assertEqual(result.stdout, "")

    def test_successful_plan_keeps_diagnostic_separate_from_json(self) -> None:
        result = self.invoke("success")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("native-nextest-diagnostic-sentinel", result.stderr)
        self.assertEqual(json.loads(result.stdout), [
            {"binary": "unica-coder", "name": "case::selected", "ignored": False},
        ])
