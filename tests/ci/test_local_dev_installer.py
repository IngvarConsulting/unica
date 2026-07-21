from __future__ import annotations

import os
import shlex
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
INSTALLER = REPO_ROOT / "scripts/dev/install-local-unica.sh"


class LocalDevInstallerTests(unittest.TestCase):
    @staticmethod
    def write_executable(path: Path, body: str) -> None:
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755)

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

    def test_windows_host_accepts_python_executable_name(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            fake_bin = tmp_path / "bin"
            fake_bin.mkdir()
            self.write_executable(
                fake_bin / "uname",
                "#!/usr/bin/env bash\n"
                "case \"$1\" in\n"
                "  -s) printf '%s\\n' 'MINGW64_NT-10.0-22631' ;;\n"
                "  -m) printf '%s\\n' 'x86_64' ;;\n"
                "  *) exit 64 ;;\n"
                "esac\n",
            )
            for name in ("python3.12", "python3.11", "python3.10", "python3"):
                self.write_executable(fake_bin / name, "#!/usr/bin/env bash\nexit 1\n")
            self.write_executable(
                fake_bin / "python",
                "#!/usr/bin/env bash\n"
                f"exec {shlex.quote(sys.executable)} \"$@\"\n",
            )
            env = os.environ.copy()
            env.pop("PYTHON", None)
            env["PATH"] = f"{fake_bin}:/usr/bin:/bin"

            completed = subprocess.run(
                [
                    str(INSTALLER),
                    "--build-dir",
                    str(tmp_path / "build"),
                    "--skip-build",
                    "--skip-install",
                    "--skip-verify",
                ],
                cwd=REPO_ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(completed.returncode, 66, completed.stderr)
        self.assertIn("==> Unica local target: win-x64\n", completed.stdout)
        self.assertIn("--skip-build requested, but bundle is missing:", completed.stderr)

    def test_prebuilt_bundle_override_controls_skip_build_check(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bundle = tmp_path / "prebuilt-bundle"
            env = os.environ.copy()
            env["UNICA_LOCAL_TOOL_BUNDLE"] = str(bundle)
            completed = subprocess.run(
                [
                    str(INSTALLER),
                    "--build-dir",
                    str(tmp_path / "build"),
                    "--skip-build",
                    "--skip-install",
                    "--skip-verify",
                ],
                cwd=REPO_ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(completed.returncode, 66)
        self.assertEqual(
            completed.stderr,
            f"--skip-build requested, but bundle is missing: {bundle}/tools.json\n",
        )

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

    def test_windows_ci_runs_local_installer_package_install_verify_smoke(self) -> None:
        workflow = (
            REPO_ROOT / ".github/workflows/unica-plugin-release.yml"
        ).read_text(encoding="utf-8")
        required = (
            "Smoke local development installer on Windows",
            "if: matrix.target == 'win-x64'",
            'CODEX_HOME="$build_root/codex-home"',
            'UNICA_LOCAL_TOOL_BUNDLE="$PWD/.build/tool-bundles/win-x64"',
            'PATH="$fake_bin:$PATH"',
            "scripts/dev/install-local-unica.sh",
            "--skip-build",
        )
        for value in required:
            with self.subTest(value=value):
                self.assertIn(value, workflow)
        self.assertNotIn("unica-tools-", workflow)


if __name__ == "__main__":
    unittest.main()
