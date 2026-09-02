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

    def test_installer_rejects_python_3_11(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            fake_bin = tmp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python-3.11"
            self.write_executable(
                fake_python,
                "#!/usr/bin/env bash\n"
                "{ printf '%s\\n' 'import sys; sys.version_info = (3, 11)'; cat; } | "
                f"{shlex.quote(sys.executable)} \"$@\"\n",
            )
            for name in (
                "python3.12",
                "python3.11",
                "python3.10",
                "python3",
                "python",
            ):
                self.write_executable(fake_bin / name, "#!/usr/bin/env bash\nexit 1\n")
            env = os.environ.copy()
            env["PYTHON"] = str(fake_python)
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

        self.assertEqual(completed.returncode, 69, completed.stderr)
        self.assertEqual(completed.stdout, "")
        self.assertIn("Python >= 3.12 is required", completed.stderr)

    def test_installer_does_not_expose_external_bundle_override(self) -> None:
        installer = INSTALLER.read_text(encoding="utf-8")
        self.assertNotIn("UNICA_LOCAL_TOOL_BUNDLE", installer)

    def test_installer_creates_codex_home_before_invoking_cli(self) -> None:
        installer = INSTALLER.read_text(encoding="utf-8")
        create_home = installer.index('mkdir -p "$CODEX_HOME_DIR"')
        first_codex_call = installer.index(
            'codex plugin marketplace remove "$MARKETPLACE_NAME"'
        )
        self.assertLess(create_home, first_codex_call)

    def test_prompt_verification_requires_only_installed_visible_skills(self) -> None:
        installer = INSTALLER.read_text(encoding="utf-8")

        # The proof looks for the plugin name and one skill taken from the
        # package at run time, so no skill name is written into the script.
        needle_lines = [
            line.strip() for line in installer.splitlines() if "for needle in" in line
        ]
        self.assertEqual(needle_lines, ['for needle in "Unica" "$PROMPT_SKILL"; do'])
        self.assertIn(
            'PROMPT_SKILL="$(packaged_prompt_skill "$MARKETPLACE_DIR/plugins/unica/skills")"',
            installer,
        )
        for stale in (
            'for needle in "Unica" "v8-runner" "db-auth-check"; do',
            'for needle in "Unica" "db-auth-check"; do',
        ):
            with self.subTest(stale=stale):
                self.assertNotIn(stale, installer)

    def packaged_prompt_skill(self, skills_root: Path) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as tmp:
            return subprocess.run(
                [
                    "bash",
                    "-c",
                    (
                        'installer="$1"; skills_root="$2"; tmp="$3"; '
                        'set -- --build-dir "$tmp" --skip-build --skip-install '
                        '--skip-verify; source "$installer"; '
                        'packaged_prompt_skill "$skills_root"'
                    ),
                    "bash",
                    str(INSTALLER),
                    str(skills_root),
                    tmp,
                ],
                cwd=REPO_ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

    def test_packaged_prompt_skill_is_a_complete_skill_of_the_package(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            skills_root = Path(tmp) / "skills"
            # A directory without SKILL.md is not a skill the host will show.
            (skills_root / "alpha").mkdir(parents=True)
            for name in ("beta", "gamma"):
                (skills_root / name).mkdir()
                (skills_root / name / "SKILL.md").write_text(f"# {name}\n", encoding="utf-8")

            completed = self.packaged_prompt_skill(skills_root)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, "beta\n")
        self.assertEqual(completed.stderr, "")

    def test_packaged_prompt_skill_refuses_a_package_without_skills(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            skills_root = Path(tmp) / "skills"
            (skills_root / "not-a-skill").mkdir(parents=True)

            completed = self.packaged_prompt_skill(skills_root)

        self.assertEqual(completed.returncode, 65)
        self.assertEqual(completed.stdout, "")
        self.assertEqual(
            completed.stderr,
            f"Packaged plugin exposes no prompt-visible skills: {skills_root}\n",
        )

    def test_packaged_prompt_skill_names_a_shipped_source_skill(self) -> None:
        skills_root = REPO_ROOT / "plugins/unica/skills"

        completed = self.packaged_prompt_skill(skills_root)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stderr, "")
        skill = completed.stdout.rstrip("\n")
        self.assertNotIn("\n", skill)
        self.assertTrue((skills_root / skill / "SKILL.md").is_file(), skill)

    def test_installer_preserves_persistent_cargo_work_and_writes_metrics(self) -> None:
        installer = INSTALLER.read_text(encoding="utf-8")
        workflow = (
            REPO_ROOT / ".github/workflows/unica-plugin-release.yml"
        ).read_text(encoding="utf-8")

        self.assertIn('WORK_DIR="$BUILD_ROOT/tool-work"', installer)
        self.assertIn(
            'METRICS_FILE="$BUILD_ROOT/cargo-metrics/$TARGET.json"', installer
        )
        self.assertIn('--metrics-file "$METRICS_FILE"', installer)
        self.assertNotIn('rm -rf "$TOOL_BUNDLE" "$WORK_DIR"', installer)

        self.assertIn('build_root="$PWD/.build"', workflow)
        self.assertNotIn('rm -rf "$build_root"', workflow)
        self.assertIn(
            "path: .build/tool-work/${{ matrix.target }}/cargo-target", workflow
        )
        self.assertIn(
            "METRICS_FILE: .build/cargo-metrics/${{ matrix.target }}.json", workflow
        )

    def test_shell_scripts_are_forced_to_lf_in_windows_checkouts(self) -> None:
        completed = subprocess.run(
            [
                "git",
                "check-attr",
                "text",
                "eol",
                "--",
                "scripts/dev/install-local-unica.sh",
            ],
            cwd=REPO_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("scripts/dev/install-local-unica.sh: text: set\n", completed.stdout)
        self.assertIn("scripts/dev/install-local-unica.sh: eol: lf\n", completed.stdout)

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
            "Python 3.12",
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
            "Build, install, and verify local development on Windows",
            "if: matrix.target == 'win-x64'",
            "uses: actions/setup-node@v7",
            "npm install --global @openai/codex@0.145.0-alpha.18",
            'codex_home="$(cygpath -m "$build_root/codex-home")"',
            'CODEX_HOME="$codex_home"',
            "scripts/dev/install-local-unica.sh",
            'if: matrix.target != \'win-x64\'',
            'find "$build_root/tool-artifacts"',
            'cp -R "$bundle_root" "$PWD/.build/tool-bundles/win-x64"',
        )
        for value in required:
            with self.subTest(value=value):
                self.assertIn(value, workflow)
        forbidden = (
            "fake_bin",
            "UNICA_LOCAL_TOOL_BUNDLE",
            "--skip-build",
            "'{\"skills\":[\"Unica\",\"v8-runner\",\"db-auth-check\"]}'",
            "unica-tools-",
        )
        for value in forbidden:
            with self.subTest(forbidden=value):
                self.assertNotIn(value, workflow)


if __name__ == "__main__":
    unittest.main()
