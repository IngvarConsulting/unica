from __future__ import annotations

import subprocess
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


def attributes(path: str) -> dict[str, str]:
    output = subprocess.run(
        ["git", "check-attr", "text", "eol", "--", path],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return {
        attribute: value
        for line in output.splitlines()
        for _, attribute, value in [line.split(": ", 2)]
    }


class CheckoutEolContractTests(unittest.TestCase):
    def test_source_scanned_and_hashed_inputs_are_lf_on_every_checkout(self) -> None:
        for path in [
            "crates/unica-coder/src/infrastructure/daemon/server.rs",
            "tests/fixtures/v013/modules/syntax-boundaries.bsl",
            "crates/unica-coder/src/infrastructure/platform-event-catalog-8.3.27.2074.json",
        ]:
            self.assertEqual(attributes(path), {"text": "set", "eol": "lf"}, path)

    def test_platform_byte_fixtures_remain_outside_text_normalization(self) -> None:
        for path in [
            "tests/fixtures/platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml",
            "tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Configuration.xml",
            "tests/fixtures/xdto/enterprise-data-minimal/Configuration.xml",
        ]:
            self.assertEqual(attributes(path)["text"], "unset", path)


if __name__ == "__main__":
    unittest.main()
