from __future__ import annotations

import subprocess
import os
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "install-unica.sh"
PS_SCRIPT = REPO_ROOT / "scripts" / "install-unica.ps1"


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


@unittest.skipIf(os.name == "nt", "install-unica.sh URL checks run on POSIX CI")
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


class InstallUnicaPowerShellScriptTests(unittest.TestCase):
    def test_windows_installer_is_power_shell_51_friendly(self) -> None:
        text = PS_SCRIPT.read_text(encoding="utf-8")

        self.assertIn('[ValidateSet("win-x64")]', text)
        self.assertIn("[Net.SecurityProtocolType]::Tls12", text)
        self.assertIn("Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing", text)
        self.assertIn("Expand-Archive -LiteralPath $archive -DestinationPath $extractDir -Force", text)
        self.assertIn('"unica-codex-marketplace-$Target.$(Get-ArchiveExtension -Target $Target)"', text)
        self.assertIn('"unica:meta-compile"', text)
        self.assertIn('"unica:v8-runner"', text)
        self.assertIn('"unica:db-auth-check"', text)
        self.assertNotIn("$IsWindows", text)
        self.assertNotIn("pwsh", text.lower())
        self.assertNotIn("bash", text.lower())

    def test_windows_installer_rewrites_installed_mcp_commands(self) -> None:
        text = PS_SCRIPT.read_text(encoding="utf-8")

        self.assertIn("function Update-UnicaMcpJson", text)
        self.assertIn("ConvertFrom-Json", text)
        self.assertIn("ConvertTo-Json -Depth 20", text)
        self.assertIn("$mcp.mcpServers.unica.command = $Command", text)
        self.assertIn("$mcp.mcpServers.unica.args = @()", text)
        self.assertIn("[System.Text.Encoding]::UTF8", text)
        self.assertIn('[string]$PluginDir = ""', text)
        self.assertIn("return [System.IO.Path]::GetFullPath($toolPath)", text)

        self.assertIn(
            '$marketplaceMcpPath = Join-Path $marketplaceDir '
            '(Join-Path "plugins" (Join-Path "unica" ".mcp.json"))',
            text,
        )
        self.assertIn(
            "Update-UnicaMcpJson -McpPath $marketplaceMcpPath -Command "
            '(Get-ToolBinary -MarketplaceDir $marketplaceDir -Target $Target -Tool "unica")',
            text,
        )
        self.assertIn('$pluginCacheMcpPath = Join-Path $pluginCacheVersionDir ".mcp.json"', text)
        self.assertIn(
            "Update-UnicaMcpJson -McpPath $pluginCacheMcpPath -Command "
            '(Get-ToolBinary -PluginDir $pluginCacheVersionDir -Target $Target -Tool "unica")',
            text,
        )
        self.assertEqual(text.count("Update-UnicaMcpJson -McpPath"), 2)

    @unittest.skipIf(os.name != "nt", "PowerShell syntax check runs on Windows")
    def test_windows_installer_print_download_url_runs_in_powershell(self) -> None:
        result = subprocess.run(
            [
                "powershell",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(PS_SCRIPT),
                "-PrintDownloadUrl",
            ],
            check=False,
            cwd=REPO_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.strip(),
            "https://github.com/IngvarConsulting/unica/releases/latest/download/"
            "unica-codex-marketplace-win-x64.zip",
        )


if __name__ == "__main__":
    unittest.main()
