"""Tests for the host preflight that runs before conveyor sessions start.

The checks read the machine only through ``Host``; a fake host stands in for
Windows, macOS and Linux so the tests do not depend on what this runner has.
"""

from __future__ import annotations

import importlib.util
import io
import json
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "host-preflight.py"
SPEC = importlib.util.spec_from_file_location("host_preflight", SCRIPT)
PREFLIGHT = importlib.util.module_from_spec(SPEC)
sys.modules["host_preflight"] = PREFLIGHT
SPEC.loader.exec_module(PREFLIGHT)

GIB = 1024 ** 3
REPO = Path("repo")
REQUIREMENTS = "lxml==6.1.1\nPyYAML==6.0.3\n"
WORKFLOW = "        with:\n          tool: cargo-nextest@0.9.148\n"
VERIFIER = 'EXPECTED_PROFILE = "x"\nEXPECTED_PLATFORM_VERSION = "8.3.27.2074"\n'
# Built as the script builds it, so the test also holds on POSIX runners.
VSWHERE = str(Path(r"C:\Program Files (x86)") / PREFLIGHT.VSWHERE)


class FakeHost(PREFLIGHT.Host):
    def __init__(self, system: str) -> None:
        self.system = system
        self.env = {"PROGRAMFILES(X86)": r"C:\Program Files (x86)"} if system == "Windows" else {}
        self.tools = {"git", "gh", "rustc", "cargo", "rustup", "cc"}
        self.tools.add("py" if system == "Windows" else "python3.12")
        self.commands = {
            ("git", "--version"): (0, "git version 2.56.0\n"),
            ("git", "config", "--get", "user.name"): (0, "Owner\n"),
            ("git", "config", "--get", "user.email"): (0, "owner@example.invalid\n"),
            ("gh", "--version"): (0, "gh version 2.103.3\n"),
            ("gh", "auth", "status"): (0, "Logged in\n"),
            ("rustc", "--version"): (0, "rustc 1.99.1\n"),
            ("cargo", "--version"): (0, "cargo 1.99.1\n"),
            ("rustup", "component", "list", "--installed"): (0, "\n".join([
                "cargo-x86_64-pc-windows-msvc", "clippy-x86_64-pc-windows-msvc",
                "rust-analyzer-aarch64-apple-darwin", "rust-src",
                "rustfmt-x86_64-unknown-linux-gnu",
            ])),
            ("cargo", "nextest", "--version"): (0, "cargo-nextest 0.9.148\n"),
            ("xcode-select", "-p"): (0, "/Library/Developer/CommandLineTools\n"),
            (VSWHERE, "-latest", "-products", "*", "-requires", PREFLIGHT.VC_TOOLS_COMPONENT,
             "-property", "installationPath"): (0, r"C:\BuildTools" + "\n"),
        }
        python = tuple(PREFLIGHT.python_command(system))
        self.commands[python + ("--version",)] = (0, "Python 3.12.10\n")
        self.packages = "lxml 6.1.1\nPyYAML 6.0.3\n"
        self.python = python
        self.files = {
            REPO / "tests" / "ci" / "requirements.txt": REQUIREMENTS,
            REPO / ".github" / "workflows" / "release.yml": WORKFLOW,
            REPO / "scripts" / "dev" / "verify-8-3-27-platform.py": VERIFIER,
        }
        self.executables = {Path(VSWHERE)} if system == "Windows" else set()
        self.dirs: dict = {}
        self.free = 120 * GIB
        self.sizes: dict = {}
        self.cores = 8
        self.memory = 32 * GIB
        self.encoding = "UTF-8"
        self.calls = []

    def which(self, name):
        return f"/bin/{name}" if name in self.tools else None

    def run(self, argv):
        argv = tuple(argv)
        self.calls.append(argv)
        if argv[: len(self.python) + 1] == self.python + ("-c",):
            return 0, self.packages
        return self.commands.get(argv, (127, "not found"))

    def free_bytes(self, path):
        return self.free

    def is_file(self, path):
        return Path(path) in self.executables

    def child_dirs(self, path):
        return self.dirs.get(Path(path), [])

    def dir_size(self, path):
        return self.sizes.get(Path(path), 0)

    def cpu_count(self):
        return self.cores

    def memory_bytes(self):
        return self.memory

    def preferred_encoding(self):
        return self.encoding

    def read_text(self, path):
        return self.files.get(Path(path))


def run(host, *args):
    return PREFLIGHT.run_checks(host, REPO, 1, list(args))


def by_name(checks):
    return {check.name: check for check in checks}


class HostPreflightTests(unittest.TestCase):
    def test_complete_hosts_are_ready_on_every_os(self) -> None:
        for system in ("Windows", "Darwin", "Linux"):
            with self.subTest(system=system):
                host = FakeHost(system)
                host.env["PYTHONUTF8"] = "1"
                checks = run(host)
                blocking = [c.name for c in checks if c.required and c.status != PREFLIGHT.OK]
                self.assertEqual(blocking, [])
                self.assertTrue(PREFLIGHT.is_ready(checks))

    def test_python_command_follows_the_os(self) -> None:
        windows = by_name(run(FakeHost("Windows")))["python 3.12"]
        linux = by_name(run(FakeHost("Linux")))["python 3.12"]
        self.assertIn("py -3.12", windows.detail)
        self.assertIn("python3.12", linux.detail)

    def test_wrong_python_version_is_missing_with_os_install_hint(self) -> None:
        host = FakeHost("Darwin")
        host.commands[("python3.12", "--version")] = (0, "Python 3.11.9\n")
        check = by_name(run(host))["python 3.12"]
        self.assertEqual(check.status, PREFLIGHT.MISSING)
        self.assertEqual(check.fix, ["brew install python@3.12"])

    def test_package_version_drift_is_reported(self) -> None:
        host = FakeHost("Linux")
        host.packages = "lxml 6.0.0\nPyYAML -\n"
        check = by_name(run(host))["python packages"]
        self.assertEqual(check.status, PREFLIGHT.MISSING)
        self.assertIn("lxml 6.0.0 (need 6.1.1)", check.detail)
        self.assertIn("PyYAML - (need 6.0.3)", check.detail)

    def test_missing_gh_auth_and_git_identity_block(self) -> None:
        host = FakeHost("Windows")
        host.env["PYTHONUTF8"] = "1"
        host.commands[("gh", "auth", "status")] = (1, "You are not logged into any GitHub hosts.\n")
        host.commands[("git", "config", "--get", "user.email")] = (1, "")
        checks = by_name(run(host))
        self.assertEqual(checks["gh auth"].fix, ["gh auth login"])
        self.assertEqual(checks["git user.email"].status, PREFLIGHT.MISSING)
        self.assertFalse(PREFLIGHT.is_ready(list(checks.values())))

    def test_windows_requires_pythonutf8(self) -> None:
        host = FakeHost("Windows")
        self.assertEqual(by_name(run(host))["PYTHONUTF8"].status, PREFLIGHT.MISSING)
        host.env["PYTHONUTF8"] = "1"
        self.assertEqual(by_name(run(host))["PYTHONUTF8"].status, PREFLIGHT.OK)

    def test_posix_requires_utf8_locale(self) -> None:
        host = FakeHost("Linux")
        host.encoding = "ANSI_X3.4-1968"
        self.assertEqual(by_name(run(host))["UTF-8 locale"].status, PREFLIGHT.MISSING)

    def test_git_link_exe_is_not_the_msvc_linker(self) -> None:
        host = FakeHost("Windows")
        host.executables = set()
        host.which = lambda name: r"C:\Program Files\Git\usr\bin\link.exe" if name == "link.exe" else None
        check = PREFLIGHT.check_c_toolchain(host)
        self.assertEqual(check.status, PREFLIGHT.MISSING)
        self.assertIn("Microsoft.VisualStudio.2022.BuildTools", check.fix[0])

    def test_missing_msvc_adds_its_size_to_the_disk_need(self) -> None:
        host = FakeHost("Windows")
        host.executables = set()
        host.free = 40 * GIB
        disk = by_name(run(host))["disk"]
        self.assertEqual(disk.status, PREFLIGHT.WARN)
        self.assertIn("7 GB for MSVC", disk.detail)

    def test_disk_scales_with_planned_copies(self) -> None:
        host = FakeHost("Linux")
        host.free = 110 * GIB
        self.assertEqual(PREFLIGHT.check_disk(host, REPO, 2, False).status, PREFLIGHT.OK)
        self.assertEqual(PREFLIGHT.check_disk(host, REPO, 4, False).status, PREFLIGHT.WARN)
        self.assertEqual(PREFLIGHT.check_disk(host, REPO, 7, False).status, PREFLIGHT.MISSING)

    def test_two_core_host_holds_one_build(self) -> None:
        host = FakeHost("Windows")
        host.cores = 2
        check = PREFLIGHT.check_capacity(host, REPO, 3)
        self.assertEqual(check.status, PREFLIGHT.WARN)
        self.assertFalse(check.required)
        self.assertIn("2 logical cores, 32.0 GB RAM: at most 1 parallel Rust build", check.detail)
        self.assertEqual(check.fix, ["keep at most 1 working copy building at once"])

    def test_build_capacity_is_the_smaller_of_cores_and_disk(self) -> None:
        host = FakeHost("Linux")
        host.cores = 16
        host.free = 110 * GIB
        check = PREFLIGHT.check_capacity(host, REPO, 2)
        self.assertEqual(check.status, PREFLIGHT.OK)
        self.assertIn("at most 2 parallel Rust builds (1 per 4 cores: 4; 50 GB free disk each: 2)", check.detail)
        host.memory = None
        self.assertIn("RAM unknown", PREFLIGHT.check_capacity(host, REPO, 1).detail)

    def test_full_disk_still_allows_one_build(self) -> None:
        host = FakeHost("Linux")
        host.free = 3 * GIB
        self.assertIn("at most 1 parallel Rust build", PREFLIGHT.check_capacity(host, REPO, 1).detail)

    def test_disk_below_the_floor_warns_even_when_copies_fit(self) -> None:
        host = FakeHost("Linux")
        host.free = 45 * GIB
        check = PREFLIGHT.check_disk(host, REPO, 1, False)
        self.assertEqual(check.status, PREFLIGHT.WARN)
        self.assertIn("below 50 GB a single test build can fill the disk", check.detail)
        host.free = 50 * GIB
        self.assertEqual(PREFLIGHT.check_disk(host, REPO, 1, False).status, PREFLIGHT.OK)

    def test_dev_debuginfo_recommends_line_tables_on_every_os(self) -> None:
        fixes = {
            "Windows": "setx CARGO_PROFILE_DEV_DEBUG line-tables-only",
            "Darwin": '$HOME/.zshrc',
            "Linux": '$HOME/.bashrc',
        }
        for system, fix in fixes.items():
            with self.subTest(system=system):
                host = FakeHost(system)
                check = by_name(run(host))["dev debuginfo"]
                self.assertEqual(check.status, PREFLIGHT.WARN)
                self.assertFalse(check.required)
                self.assertIn(fix, check.fix[0])
                host.env["CARGO_PROFILE_DEV_DEBUG"] = "line-tables-only"
                self.assertEqual(by_name(run(host))["dev debuginfo"].status, PREFLIGHT.OK)

    def test_build_dirs_are_listed_largest_first(self) -> None:
        host = FakeHost("Windows")
        root = Path(r"C:\cargo-target")
        host.dirs[root] = [root / "worker-1", root / "worker-2"]
        host.sizes = {root / "worker-1": 26 * GIB, root / "worker-2": 50 * GIB}
        check = by_name(run(host))["build dirs"]
        self.assertEqual(check.status, PREFLIGHT.OK)
        self.assertIn("76.0 GB under", check.detail)
        self.assertIn("worker-2 50.0 GB, worker-1 26.0 GB", check.detail)
        host.free = 20 * GIB
        check = by_name(run(host))["build dirs"]
        self.assertEqual(check.status, PREFLIGHT.WARN)
        self.assertEqual(check.fix, ["py -3.12 scripts/dev/prune-build-caches.py"])

    def test_build_root_follows_the_os_and_the_option(self) -> None:
        host = FakeHost("Linux")
        host.env["HOME"] = "/home/dev"
        self.assertEqual(PREFLIGHT.default_build_root(host), Path("/home/dev/cargo-target"))
        other = Path("/data/builds")
        host.dirs[other] = [other / "w1"]
        checks = PREFLIGHT.run_checks(host, REPO, 1, [], other)
        self.assertIn("w1 0.0 GB", by_name(checks)["build dirs"].detail)

    def test_package_hint_uses_venv_where_system_python_refuses_pip(self) -> None:
        host = FakeHost("Darwin")
        host.packages = ""
        fix = by_name(run(host))["python packages"].fix
        self.assertEqual(fix[0], 'python3.12 -m venv "$HOME/.venvs/unica"')
        host = FakeHost("Windows")
        host.packages = ""
        self.assertEqual(by_name(run(host))["python packages"].fix,
                         ["py -3.12 -m pip install -r tests/ci/requirements.txt"])

    def test_failed_gh_auth_names_the_reason(self) -> None:
        host = FakeHost("Linux")
        host.commands[("gh", "auth", "status")] = (1, "github.com\n  X Failed to log in to github.com using token (GH_TOKEN)\n")
        self.assertEqual(by_name(run(host))["gh auth"].detail,
                         "X Failed to log in to github.com using token (GH_TOKEN)")

    def test_non_ascii_values_do_not_break_an_ascii_console(self) -> None:
        host = FakeHost("Linux")
        host.commands[("git", "config", "--get", "user.name")] = (0, "Łukasz 李\n")
        raw = io.BytesIO()
        stream = io.TextIOWrapper(raw, encoding="ascii")
        with redirect_stdout(stream):
            PREFLIGHT.main([], host=host, repo=REPO)
        stream.flush()
        self.assertIn(b"\u0141ukasz", raw.getvalue())

    def test_missing_nextest_hint_uses_ci_pin(self) -> None:
        host = FakeHost("Linux")
        del host.commands[("cargo", "nextest", "--version")]
        check = by_name(run(host))["cargo-nextest"]
        self.assertEqual(check.fix, ["cargo install cargo-nextest --version 0.9.148 --locked"])

    def test_missing_rust_component_is_named(self) -> None:
        host = FakeHost("Linux")
        host.commands[("rustup", "component", "list", "--installed")] = (0, "rustfmt-x86_64-unknown-linux-gnu\nrust-src\n")
        check = by_name(run(host))["rust components"]
        self.assertEqual(check.detail, "missing: clippy, rust-analyzer")

    def test_pinned_platform_is_found_in_product_roots(self) -> None:
        host = FakeHost("Linux")
        root = Path("/opt/1cv8/x86_64")
        host.dirs[root] = [root / "8.3.24.1000", root / "8.3.27.2074"]
        host.executables = {root / "8.3.24.1000" / "ibcmd", root / "8.3.27.2074" / "ibcmd"}
        check = by_name(run(host))["1C platform"]
        self.assertEqual(check.status, PREFLIGHT.OK)
        self.assertTrue(check.detail.startswith("8.3.27.2074"))

    def test_windows_platform_uses_bin_and_reports_other_versions(self) -> None:
        host = FakeHost("Windows")
        root = Path(r"C:\Program Files\1cv8")
        host.dirs[root] = [root / "8.3.27.2342", root / "common"]
        host.executables.add(root / "8.3.27.2342" / "bin" / "ibcmd.exe")
        check = by_name(run(host))["1C platform"]
        self.assertEqual(check.status, PREFLIGHT.WARN)
        self.assertEqual(check.detail, "found 8.3.27.2342; pinned 8.3.27.2074 is absent")
        self.assertFalse(check.required)

    def test_missing_platform_does_not_block_readiness(self) -> None:
        host = FakeHost("Darwin")
        checks = run(host)
        self.assertEqual(by_name(checks)["1C platform"].status, PREFLIGHT.WARN)
        self.assertTrue(PREFLIGHT.is_ready(checks))

    def test_extra_platform_root_is_searched(self) -> None:
        host = FakeHost("Darwin")
        root = Path("/Applications/1cv8")
        host.dirs[root] = [root / "8.3.27.2074"]
        host.executables = {root / "8.3.27.2074" / "ibcmd"}
        self.assertEqual(by_name(run(host, str(root)))["1C platform"].status, PREFLIGHT.OK)

    def test_text_report_is_ascii_and_exit_code_follows_readiness(self) -> None:
        host = FakeHost("Windows")
        out = io.StringIO()
        with redirect_stdout(out):
            code = PREFLIGHT.main([], host=host, repo=REPO)
        text = out.getvalue()
        self.assertEqual(code, 1)
        text.encode("ascii")
        self.assertIn("[MISS] PYTHONUTF8", text)
        self.assertIn("-> setx PYTHONUTF8 1", text)
        self.assertTrue(text.rstrip().endswith("Not ready: PYTHONUTF8"))

    def test_json_report(self) -> None:
        host = FakeHost("Linux")
        out = io.StringIO()
        with redirect_stdout(out):
            code = PREFLIGHT.main(["--json"], host=host, repo=REPO)
        report = json.loads(out.getvalue())
        self.assertEqual(code, 0)
        self.assertTrue(report["ready"])
        self.assertEqual(report["system"], "Linux")
        self.assertIn("python 3.12", [check["name"] for check in report["checks"]])

    def test_checks_never_install_anything(self) -> None:
        host = FakeHost("Windows")
        run(host)
        for argv in host.calls:
            self.assertNotIn("install", argv, argv)
            self.assertNotIn("add", argv, argv)


if __name__ == "__main__":
    unittest.main()
