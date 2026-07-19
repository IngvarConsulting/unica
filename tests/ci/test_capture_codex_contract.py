from __future__ import annotations

import json
import importlib.util
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "capture-codex-contract.py"


def load_capture_module():
    spec = importlib.util.spec_from_file_location("capture_codex_contract", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@unittest.skipIf(os.name == "nt", "fake executable fixture uses a POSIX launcher")
class CaptureCodexContractTests(unittest.TestCase):
    def test_captures_versioned_sanitized_contract_atomically(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            codex_home = root / "codex-home"
            codex_home.mkdir()
            fake_codex = root / "codex"
            output = root / "contract"
            self.write_fake_codex(fake_codex)

            result = subprocess.run(
                [
                    "python3.12",
                    str(SCRIPT),
                    "--codex",
                    str(fake_codex),
                    "--codex-home",
                    str(codex_home),
                    "--expected-version",
                    "codex-cli 0.145.0-alpha.18",
                    "--output-dir",
                    str(output),
                ],
                cwd=REPO_ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            marketplaces = json.loads((output / "marketplaces.json").read_text())
            plugins = json.loads((output / "plugins.json").read_text())
            metadata = json.loads((output / "metadata.json").read_text())
            rendered = "\n".join(
                path.read_text(encoding="utf-8") for path in sorted(output.iterdir())
            )

            self.assertEqual(
                marketplaces["marketplaces"][0]["root"],
                "${CODEX_HOME}/marketplaces/unica-local",
            )
            self.assertEqual(
                plugins["installed"][0]["source"]["path"],
                "${CODEX_HOME}/marketplaces/unica-local/plugins/unica",
            )
            self.assertEqual(metadata["codexVersion"], "codex-cli 0.145.0-alpha.18")
            self.assertEqual(
                metadata["commands"],
                [
                    "codex plugin marketplace list --json",
                    "codex plugin list --available --json",
                ],
            )
            self.assertNotIn(str(codex_home), rendered)
            self.assertFalse(any(root.glob(".contract.tmp-*")))

    def test_invalid_json_leaves_no_partial_contract(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            codex_home = root / "codex-home"
            codex_home.mkdir()
            fake_codex = root / "codex"
            output = root / "contract"
            self.write_fake_codex(fake_codex)
            env = os.environ.copy()
            env["FAKE_CODEX_BAD_JSON"] = "1"

            result = subprocess.run(
                [
                    "python3.12",
                    str(SCRIPT),
                    "--codex",
                    str(fake_codex),
                    "--codex-home",
                    str(codex_home),
                    "--expected-version",
                    "codex-cli 0.145.0-alpha.18",
                    "--output-dir",
                    str(output),
                ],
                cwd=REPO_ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("marketplace JSON", result.stderr)
            self.assertFalse(output.exists())
            self.assertFalse(any(root.glob(".contract.tmp-*")))

    def test_external_absolute_path_is_rejected_instead_of_leaked(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            codex_home = root / "codex-home"
            codex_home.mkdir()
            fake_codex = root / "codex"
            output = root / "contract"
            self.write_fake_codex(fake_codex)
            env = os.environ.copy()
            env["FAKE_CODEX_EXTERNAL_PATH"] = "1"

            result = subprocess.run(
                [
                    "python3.12",
                    str(SCRIPT),
                    "--codex",
                    str(fake_codex),
                    "--codex-home",
                    str(codex_home),
                    "--expected-version",
                    "codex-cli 0.145.0-alpha.18",
                    "--output-dir",
                    str(output),
                ],
                cwd=REPO_ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("outside CODEX_HOME", result.stderr)
            self.assertNotIn("/Users/private-marketplace", result.stderr)
            self.assertFalse(output.exists())

    def test_shared_prefix_path_is_not_mistaken_for_codex_home(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            codex_home = root / "codex-home"
            codex_home.mkdir()
            fake_codex = root / "codex"
            output = root / "contract"
            self.write_fake_codex(fake_codex)
            env = os.environ.copy()
            env["FAKE_CODEX_PREFIX_PATH"] = "1"

            result = subprocess.run(
                [
                    "python3.12",
                    str(SCRIPT),
                    "--codex",
                    str(fake_codex),
                    "--codex-home",
                    str(codex_home),
                    "--expected-version",
                    "codex-cli 0.145.0-alpha.18",
                    "--output-dir",
                    str(output),
                ],
                cwd=REPO_ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("outside CODEX_HOME", result.stderr)
            self.assertNotIn(f"{codex_home}-secret", result.stderr)
            self.assertFalse(output.exists())

    def test_wrong_or_path_bearing_version_is_rejected_without_leak(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            codex_home = root / "codex-home"
            codex_home.mkdir()
            fake_codex = root / "codex"
            output = root / "contract"
            self.write_fake_codex(fake_codex)
            env = os.environ.copy()
            env["FAKE_CODEX_VERSION"] = "not-codex /Users/version-secret"

            result = subprocess.run(
                [
                    "python3.12",
                    str(SCRIPT),
                    "--codex",
                    str(fake_codex),
                    "--codex-home",
                    str(codex_home),
                    "--expected-version",
                    "codex-cli 0.145.0-alpha.18",
                    "--output-dir",
                    str(output),
                ],
                cwd=REPO_ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("does not match --expected-version", result.stderr)
            self.assertNotIn("/Users/version-secret", result.stderr)
            self.assertFalse(output.exists())

    def test_windows_root_sanitization_is_case_insensitive_and_separator_aware(self) -> None:
        module = load_capture_module()

        self.assertEqual(
            module.sanitize_text(
                r"c:/users/test/.codex/marketplaces/unica",
                Path(r"C:\Users\Test\.codex"),
            ),
            r"${CODEX_HOME}/marketplaces/unica",
        )
        self.assertEqual(
            module.sanitize_text(
                r"C:\Users\Test\.codex-secret\marketplace",
                Path(r"C:\Users\Test\.codex"),
            ),
            r"C:\Users\Test\.codex-secret\marketplace",
        )

    @staticmethod
    def write_fake_codex(path: Path) -> None:
        path.write_text(
            r'''#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' "${FAKE_CODEX_VERSION:-codex-cli 0.145.0-alpha.18}"
elif [ "$2" = "marketplace" ]; then
  if [ "${FAKE_CODEX_BAD_JSON:-}" = "1" ]; then
    printf '%s\n' '{bad json'
  elif [ "${FAKE_CODEX_EXTERNAL_PATH:-}" = "1" ]; then
    printf '%s\n' '{"marketplaces":[{"root":"/Users/private-marketplace","name":"private"}]}'
  elif [ "${FAKE_CODEX_PREFIX_PATH:-}" = "1" ]; then
    printf '{"marketplaces":[{"root":"%s-secret/marketplace","name":"private"}]}\n' "$CODEX_HOME"
  else
    printf '{"marketplaces":[{"root":"%s/marketplaces/unica-local","name":"unica"}]}\n' "$CODEX_HOME"
  fi
else
  printf '{"installed":[{"source":{"path":"%s/marketplaces/unica-local/plugins/unica"}}],"available":[]}\n' "$CODEX_HOME"
fi
''',
            encoding="utf-8",
        )
        path.chmod(path.stat().st_mode | stat.S_IXUSR)


if __name__ == "__main__":
    unittest.main()
