from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "install-unica.sh"
PS_SCRIPT = REPO_ROOT / "scripts" / "install-unica.ps1"


@unittest.skipIf(os.name == "nt", "POSIX shim behavior runs on POSIX CI")
class InstallUnicaScriptTests(unittest.TestCase):
    def test_help_describes_git_marketplace_migration(self) -> None:
        result = subprocess.run(
            [str(SCRIPT), "--help"],
            cwd=REPO_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("IngvarConsulting/unica-marketplace", result.stdout)
        self.assertIn("migration backup", result.stdout.lower())
        self.assertNotIn("release asset", result.stdout.lower())

    def test_shim_clones_stable_catalog_then_runs_preflight_before_migration(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            log = root / "calls.log"
            codex_home = root / "codex-home"
            self.write_fake_git(fake_bin / "git")
            self.write_executable(fake_bin / "codex", "#!/bin/sh\nexit 0\n")
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "UNICA_TEST_LOG": str(log),
                    "CODEX_HOME": str(codex_home),
                }
            )

            result = subprocess.run(
                [str(SCRIPT), "--target", "linux-x64"],
                cwd=REPO_ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            calls = log.read_text(encoding="utf-8").splitlines()
            self.assertIn(
                "git clone --depth 1 --branch main "
                "https://github.com/IngvarConsulting/unica-marketplace.git",
                calls[1],
            )
            self.assertTrue(any("fetch --depth 1 origin refs/tags/v0.7.3" in line for line in calls))
            bootstrap_calls = [line for line in calls if line.startswith("bootstrap ")]
            self.assertEqual(
                bootstrap_calls,
                [
                    f"bootstrap migrate-preflight CODEX_HOME={codex_home}",
                    f"bootstrap migrate CODEX_HOME={codex_home}",
                ],
            )
            self.assertIn("Migration backup", result.stdout)
            self.assertIn("Open a new Codex task or restart the client", result.stdout)

    def test_shim_contains_no_legacy_archive_or_manual_config_mutation(self) -> None:
        text = SCRIPT.read_text(encoding="utf-8")

        for forbidden in (
            "curl",
            "wget",
            "unica-codex-marketplace-",
            "config.toml",
            "plugins/cache",
            "enable_codex_plugin",
            "node",
            "python",
        ):
            self.assertNotIn(forbidden, text.lower())
        self.assertIn('MARKETPLACE_REF="${UNICA_MARKETPLACE_REF:-main}"', text)
        self.assertIn("migrate-preflight", text)
        self.assertIn("migrate", text)
        self.assertIn("Open a new Codex task or restart the client", text)

    def write_fake_git(self, path: Path) -> None:
        self.write_executable(
            path,
            r'''#!/bin/sh
printf 'git %s\n' "$*" >> "$UNICA_TEST_LOG"
if [ "$1" = "clone" ]; then
  eval "destination=\${$#}"
  mkdir -p "$destination/.agents/plugins"
  mkdir -p "$destination/plugins/unica/bootstrap/bin/linux-x64"
  printf '%s\n' '{"name":"unica","plugins":[{"name":"unica","source":{"source":"git-subdir","url":"https://github.com/IngvarConsulting/unica-marketplace.git","path":"./plugins/unica","ref":"v0.7.3"}}]}' > "$destination/.agents/plugins/marketplace.json"
  cat > "$destination/plugins/unica/bootstrap/bin/linux-x64/unica-bootstrap" <<'BOOTSTRAP'
#!/bin/sh
printf 'bootstrap %s CODEX_HOME=%s\n' "$1" "$CODEX_HOME" >> "$UNICA_TEST_LOG"
if [ "$1" = "migrate-preflight" ]; then
  printf '%s\n' '{"addCanonicalMarketplace":true}'
else
  printf '%s\n' '{"changed":true,"backupDir":"/backup/unica"}'
fi
BOOTSTRAP
  chmod +x "$destination/plugins/unica/bootstrap/bin/linux-x64/unica-bootstrap"
fi
exit 0
''',
        )

    def write_executable(self, path: Path, text: str) -> None:
        path.write_text(text, encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)


class InstallUnicaPowerShellScriptTests(unittest.TestCase):
    def test_windows_shim_is_powershell_51_git_only_and_transactional(self) -> None:
        text = PS_SCRIPT.read_text(encoding="utf-8")
        lower = text.lower()

        self.assertIn('[ValidateSet("win-x64")]', text)
        self.assertIn("ConvertFrom-Json", text)
        self.assertIn("IngvarConsulting/unica-marketplace", text)
        self.assertIn("unica-bootstrap.exe", text)
        self.assertIn('"migrate-preflight"', text)
        self.assertIn('"migrate"', text)
        self.assertIn("Open a new Codex task or restart the client", text)
        self.assertNotIn("pwsh", lower)
        self.assertNotIn("bash", lower)
        self.assertNotIn("invoke-webrequest", lower)
        self.assertNotIn("expand-archive", lower)
        self.assertNotIn("config.toml", lower)
        self.assertNotIn("plugins\\cache", lower)
        self.assertNotIn("node", lower)

    @unittest.skipIf(os.name != "nt", "PowerShell behavior runs on Windows CI")
    def test_windows_help_runs_in_windows_powershell(self) -> None:
        result = subprocess.run(
            [
                "powershell",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(PS_SCRIPT),
                "-Help",
            ],
            cwd=REPO_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("unica-marketplace", result.stdout)


if __name__ == "__main__":
    unittest.main()
