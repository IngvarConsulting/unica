from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
# Byte fixtures that tests read, splice or hash: every file must have its line
# endings pinned, so a Windows checkout cannot hand them over with CRLF (#1988).
PINNED_FIXTURE_ROOTS = (
    "tests/fixtures/acceptance",
    "tests/fixtures/platform_8_3_27/mxl",
    "tests/fixtures/unica_mcp_script_parity/dcs-validate",
)


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


def unpinned_line_endings(root: Path, prefixes: tuple[str, ...]) -> list[str]:
    """Tracked files under `prefixes` whose checkout line endings are not pinned."""
    paths = subprocess.run(
        ["git", "ls-files", "-z", "--", *prefixes],
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    paths = [path for path in paths if path]
    if not paths:
        return []
    output = subprocess.run(
        ["git", "check-attr", "--stdin", "-z", "text", "eol"],
        cwd=root,
        check=True,
        capture_output=True,
        input=b"\0".join(paths) + b"\0",
    ).stdout.split(b"\0")
    attrs: dict[str, dict[str, str]] = {}
    for path, attribute, value in zip(output[0::3], output[1::3], output[2::3]):
        attrs.setdefault(path.decode(), {})[attribute.decode()] = value.decode()
    return sorted(
        path
        for path, values in attrs.items()
        if not (values["text"] == "unset" or (values["text"] == "set" and values["eol"] == "lf"))
    )


class CheckoutEolContractTests(unittest.TestCase):
    def test_source_scanned_and_hashed_inputs_are_lf_on_every_checkout(self) -> None:
        for path in [
            "crates/unica-coder/src/infrastructure/daemon/server.rs",
            "tests/fixtures/v013/modules/syntax-boundaries.bsl",
            "crates/unica-metadata/src/platform-event-catalog-8.3.27.2074.json",
        ]:
            self.assertEqual(attributes(path), {"text": "set", "eol": "lf"}, path)

    def test_platform_byte_fixtures_remain_outside_text_normalization(self) -> None:
        for path in [
            "tests/fixtures/platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml",
            "tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Configuration.xml",
            "tests/fixtures/xdto/enterprise-data-minimal/Configuration.xml",
        ]:
            self.assertEqual(attributes(path)["text"], "unset", path)

    def test_byte_fixtures_pin_their_line_endings(self) -> None:
        self.assertEqual(unpinned_line_endings(REPO_ROOT, PINNED_FIXTURE_ROOTS), [])

    def test_a_new_fixture_without_pinned_line_endings_is_refused(self) -> None:
        # The rule of the repository is copied into a scratch repository, so a
        # fixture that escapes it is caught however it escapes.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            (root / ".gitattributes").write_bytes((REPO_ROOT / ".gitattributes").read_bytes())
            files = {
                "tests/fixtures/acceptance/new-fixture/src/Module.bsl": "covered by the root rule\n",
                "tests/fixtures/acceptance/escaped/src/Form.xml": "escaped by a local rule\n",
                "tests/fixtures/acceptance/escaped/.gitattributes": "* text=auto\n",
                "tests/fixtures/other/Module.bsl": "outside the pinned roots\n",
            }
            for relative, text in files.items():
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8")
            subprocess.run(["git", "add", "-A"], cwd=root, check=True)
            self.assertEqual(
                unpinned_line_endings(root, PINNED_FIXTURE_ROOTS),
                [
                    "tests/fixtures/acceptance/escaped/.gitattributes",
                    "tests/fixtures/acceptance/escaped/src/Form.xml",
                ],
            )


if __name__ == "__main__":
    unittest.main()
