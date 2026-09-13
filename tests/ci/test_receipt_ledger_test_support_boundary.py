"""Страж границы признака `receipt-ledger-test-support`: только элементы.

Правило `INV.TEST.LEDGER-SUPPORT-GATES-ITEMS`: атрибут признака висит на
элементах модуля, никогда на операторе, выражении, аргументе или поле;
`not(feature = ...)` запрещён. Проверяется и на дереве, и на синтетических
исходниках, чтобы страж падал ровно на том, что запрещено.
"""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "ci" / "check-receipt-ledger-test-support-boundary.py"
SOURCE_PATH = Path("crates/unica-coder/src/infrastructure/daemon/runtime_v5.rs")

ATTR = '#[cfg(feature = "receipt-ledger-test-support")]'


def write_source(root: Path, source: str) -> None:
    path = root / SOURCE_PATH
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source, encoding="utf-8")


def run_guard(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["python3", str(SCRIPT_PATH), "--root", str(root)],
        text=True,
        capture_output=True,
        check=False,
    )


class ReceiptLedgerTestSupportBoundaryTests(unittest.TestCase):
    def test_feature_attributes_gate_items_not_statements(self) -> None:
        """Живое дерево: ни одного признака на операторе и ни одного `not(feature)`."""
        result = run_guard(REPO_ROOT)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_items_pass(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_source(
                root,
                f"{ATTR}\nmod harness;\n"
                f"{ATTR}\npub(crate) use harness::run;\n"
                f"{ATTR}\n#[allow(dead_code)]\nfn probe_for_test() {{}}\n"
                f"#[cfg(any(test, feature = \"receipt-ledger-test-support\"))]\n"
                f"/// A seed.\npub(crate) struct Seed;\n"
                f"{ATTR}\nimpl Seed {{}}\n",
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_statement_expression_field_and_arm_fail(self) -> None:
        cases = {
            "statement": f"fn run() {{\n    {ATTR}\n    let owns = true;\n}}\n",
            "expression": f"fn run() {{\n    call(\n        {ATTR}\n        &self.telemetry,\n    );\n}}\n",
            "field": f"struct Runtime {{\n    {ATTR}\n    telemetry: Telemetry,\n}}\n",
            "parameter": f"fn run(\n    {ATTR} telemetry: &Telemetry,\n) {{}}\n",
            "arm": f"fn run() {{\n    match x {{\n        {ATTR}\n        Reply::FailStop(r) => {{}}\n    }}\n}}\n",
            "block": f"fn run() {{\n    {ATTR}\n    {{\n        record();\n    }}\n}}\n",
        }
        for label, source in cases.items():
            with self.subTest(case=label), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                write_source(root, source)
                result = run_guard(root)
                self.assertEqual(result.returncode, 1, f"{label}: {result.stdout}")
                self.assertIn("not an item", result.stdout)

    def test_not_feature_is_forbidden_even_on_items(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_source(
                root,
                '#[cfg(not(feature = "receipt-ledger-test-support"))]\nfn production_only() {}\n',
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("forbidden", result.stdout)

    def test_cfg_attr_with_not_feature_is_forbidden(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_source(
                root,
                '#[cfg_attr(not(feature = "receipt-ledger-test-support"), allow(dead_code))]\nfn f() {}\n',
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 1, result.stdout)


if __name__ == "__main__":
    unittest.main()
