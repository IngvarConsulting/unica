from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
INSTALLER = REPO_ROOT / "scripts/dev/install-local-unica.sh"


class LocalDevInstallerTests(unittest.TestCase):
    def target_for_host(self, system: str, machine: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as tmp:
            return subprocess.run(
                [
                    "bash",
                    "-c",
                    (
                        'installer="$1"; system="$2"; machine="$3"; tmp="$4"; '
                        'set -- --build-dir "$tmp" --skip-build --skip-install '
                        '--skip-verify; source "$installer"; '
                        'target_for_host "$system" "$machine"'
                    ),
                    "bash",
                    str(INSTALLER),
                    system,
                    machine,
                    tmp,
                ],
                cwd=REPO_ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

    def test_supported_hosts_map_to_package_targets(self) -> None:
        cases = (
            ("Darwin", "arm64", "darwin-arm64"),
            ("Linux", "x86_64", "linux-x64"),
            ("Linux", "amd64", "linux-x64"),
            ("MINGW64_NT-10.0-19045", "x86_64", "win-x64"),
            ("MINGW64_NT-10.0-22631", "amd64", "win-x64"),
        )
        for system, machine, expected in cases:
            with self.subTest(system=system, machine=machine):
                completed = self.target_for_host(system, machine)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assertEqual(completed.stdout, expected + "\n")
                self.assertEqual(completed.stderr, "")

    def test_wsl_keeps_linux_semantics(self) -> None:
        completed = self.target_for_host("Linux", "x86_64")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, "linux-x64\n")

    def test_unsupported_shells_and_hosts_keep_status_78(self) -> None:
        cases = (
            ("MSYS_NT-10.0-19045", "x86_64"),
            ("CYGWIN_NT-10.0-19045", "x86_64"),
            ("MINGW64_NT-10.0-19045", "aarch64"),
            ("FreeBSD", "x86_64"),
        )
        for system, machine in cases:
            with self.subTest(system=system, machine=machine):
                completed = self.target_for_host(system, machine)
                self.assertEqual(completed.returncode, 78)
                self.assertEqual(completed.stdout, "")
                self.assertEqual(
                    completed.stderr,
                    f"Unsupported local Unica tool target: {system}-{machine}\n",
                )

    def test_windows_local_development_docs_name_shell_and_prerequisites(self) -> None:
        required = (
            "Git Bash",
            "Python 3.10",
            "MSVC",
            "Microsoft C++ Build Tools",
            "Windows SDK",
            "scripts/dev/install-local-unica.sh",
            "WSL",
            "MSYS2",
            "Cygwin",
        )
        for relative_path in ("README.md", "plugins/unica/README.md"):
            text = (REPO_ROOT / relative_path).read_text(encoding="utf-8")
            for value in required:
                with self.subTest(path=relative_path, value=value):
                    self.assertIn(value, text)


if __name__ == "__main__":
    unittest.main()
