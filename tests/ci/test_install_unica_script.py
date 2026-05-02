from __future__ import annotations

import subprocess
import os
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "install-unica.sh"


def script_command(*args: str) -> list[str]:
    if os.name == "nt":
        return ["bash", "./scripts/install-unica.sh", *args]
    return [str(SCRIPT), *args]


def run_script(*args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        script_command(*args),
        check=False,
        cwd=REPO_ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


class InstallUnicaScriptTests(unittest.TestCase):
    def test_prints_latest_release_asset_url_for_target(self) -> None:
        result = run_script("--target", "darwin-arm64", "--print-download-url")

        self.assertEqual(result.returncode, 0, result.stderr)

        self.assertEqual(
            result.stdout.strip(),
            "https://github.com/IngvarConsulting/unica/releases/latest/download/"
            "unica-codex-marketplace-darwin-arm64.tar.gz",
        )

    def test_prints_pinned_release_asset_url_for_target(self) -> None:
        result = run_script("--target", "linux-x64", "--version", "v0.3.3", "--print-download-url")

        self.assertEqual(result.returncode, 0, result.stderr)

        self.assertEqual(
            result.stdout.strip(),
            "https://github.com/IngvarConsulting/unica/releases/download/v0.3.3/"
            "unica-codex-marketplace-linux-x64.tar.gz",
        )

    def test_print_download_url_does_not_require_codex_home(self) -> None:
        env = os.environ.copy()
        env.pop("CODEX_HOME", None)
        env.pop("HOME", None)
        env.pop("USERPROFILE", None)

        result = run_script("--target", "win-x64", "--print-download-url", env=env)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.strip(),
            "https://github.com/IngvarConsulting/unica/releases/latest/download/"
            "unica-codex-marketplace-win-x64.zip",
        )


if __name__ == "__main__":
    unittest.main()
