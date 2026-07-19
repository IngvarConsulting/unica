from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "check-version-contract.py"


def load_module():
    spec = importlib.util.spec_from_file_location("check_version_contract", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class VersionContractTests(unittest.TestCase):
    def test_repository_versions_are_exactly_0_8_0(self) -> None:
        module = load_module()

        values = module.read_version_contract(REPO_ROOT)

        self.assertEqual(
            values,
            {
                "workspace": "0.8.0",
                "plugin": "0.8.0",
                "tools-lock-unica": "0.8.0",
            },
        )

    def test_mismatch_names_the_contract_field(self) -> None:
        module = load_module()

        errors = module.validate_version_contract(
            {"workspace": "0.7.0", "plugin": "0.6.1"},
            expected="0.7.0",
        )

        self.assertEqual(errors, ["plugin version 0.6.1 != expected 0.7.0"])


if __name__ == "__main__":
    unittest.main()
