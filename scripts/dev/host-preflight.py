#!/usr/bin/env python3
"""Check that this host can run Unica development and conveyor sessions.

One command reports what is ready, what is missing and how to install it on
the current OS (Windows, macOS or Linux). Nothing is installed or changed.

The script runs on any Python 3.8+, so it can report a missing Python 3.12.
Its output is ASCII so a cp1251 console without PYTHONUTF8 can still print it.
"""

from __future__ import annotations

import argparse
import json
import locale
import os
import platform
import re
import shutil
import subprocess
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple


REPO_ROOT = Path(__file__).resolve().parents[2]
GIB = 1024 ** 3
# A working copy with its Cargo build directory takes 25-50 GB once the
# workspace tests have been built.
COPY_MIN_GIB = 25
COPY_COMFORT_GIB = 50
# Below this much free space a single test build can fill the disk.
LOW_FREE_GIB = 50
# Line tables keep file and line numbers in panics and backtraces but drop
# variable debug info, the bulk of a dev build directory.
DEV_DEBUG = "line-tables-only"
# One parallel Rust build per 4 logical cores: on a 2-core host three builds
# starved each other.
CORES_PER_BUILD = 4
MSVC_BUILD_TOOLS_GIB = 7
RUST_COMPONENTS = ("rustfmt", "clippy", "rust-analyzer", "rust-src")
# Same roots as `default_platform_roots` in
# crates/unica-adapter-v8-runner/src/full_dump.rs; the product has no others.
PLATFORM_ROOTS = {
    "Windows": (r"C:\Program Files\1cv8", r"C:\Program Files (x86)\1cv8"),
    "Linux": ("/opt/1cv8/x86_64", "/opt/1cv8/i386", "/usr/local/1cv8"),
    "Darwin": ("/opt/1cv8",),
}
# `rustup component list --installed` prints `clippy-x86_64-pc-windows-msvc`.
TARGET_SUFFIX = re.compile(r"-(?:x86_64|aarch64|i686|arm[a-z0-9]*|riscv64[a-z0-9]*|powerpc64le|s390x|loongarch64)-.*$")
VSWHERE = r"Microsoft Visual Studio\Installer\vswhere.exe"
VC_TOOLS_COMPONENT = "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"

OK = "ok"
WARN = "warn"
MISSING = "missing"


@dataclass
class Check:
    name: str
    status: str
    detail: str
    fix: List[str] = field(default_factory=list)
    required: bool = True


class Host:
    """Everything the checks read from the machine; tests replace it."""

    def __init__(self) -> None:
        self.system = platform.system()
        # Upper-case keys: Windows names are case-insensitive (`ProgramFiles(x86)`).
        self.env: Dict[str, str] = {key.upper(): value for key, value in os.environ.items()}

    def which(self, name: str) -> Optional[str]:
        return shutil.which(name)

    def run(self, argv: Sequence[str]) -> Tuple[int, str]:
        try:
            completed = subprocess.run(
                list(argv),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=60,
            )
        except (OSError, subprocess.SubprocessError) as error:
            return 127, str(error)
        return completed.returncode, completed.stdout.decode("utf-8", "replace")

    def free_bytes(self, path: Path) -> int:
        return shutil.disk_usage(str(path)).free

    def is_file(self, path: Path) -> bool:
        return path.is_file()

    def child_dirs(self, path: Path) -> List[Path]:
        try:
            return sorted(child for child in path.iterdir() if child.is_dir())
        except OSError:
            return []

    def dir_size(self, path: Path) -> int:
        total = 0
        stack = [str(path)]
        while stack:
            try:
                entries = list(os.scandir(stack.pop()))
            except OSError:
                continue
            for entry in entries:
                try:
                    if entry.is_dir(follow_symlinks=False):
                        stack.append(entry.path)
                    else:
                        total += entry.stat(follow_symlinks=False).st_size
                except OSError:
                    continue
        return total

    def cpu_count(self) -> int:
        return os.cpu_count() or 1

    def memory_bytes(self) -> Optional[int]:
        if self.system == "Windows":
            import ctypes

            class MemoryStatus(ctypes.Structure):
                _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong),
                            ("total", ctypes.c_ulonglong), ("available", ctypes.c_ulonglong),
                            ("page_total", ctypes.c_ulonglong), ("page_available", ctypes.c_ulonglong),
                            ("virtual_total", ctypes.c_ulonglong), ("virtual_available", ctypes.c_ulonglong),
                            ("extended", ctypes.c_ulonglong)]

            status = MemoryStatus()
            status.length = ctypes.sizeof(MemoryStatus)
            if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
                return int(status.total)
            return None
        try:
            return os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
        except (AttributeError, OSError, ValueError):
            return None

    def preferred_encoding(self) -> str:
        return locale.getpreferredencoding(False)

    def read_text(self, path: Path) -> Optional[str]:
        try:
            return path.read_text(encoding="utf-8")
        except OSError:
            return None


def by_os(system: str, windows: str, macos: str, linux: str) -> str:
    return {"Windows": windows, "Darwin": macos}.get(system, linux)


def first_line(text: str) -> str:
    for line in text.splitlines():
        if line.strip():
            return line.strip()
    return ""


def python_command(system: str) -> List[str]:
    """The Python 3.12 command the development skills mean."""
    return ["py", "-3.12"] if system == "Windows" else ["python3.12"]


def check_gh(host: Host) -> List[Check]:
    fix = by_os(
        host.system,
        "winget install --id GitHub.cli -e",
        "brew install gh",
        "see https://github.com/cli/cli/blob/trunk/docs/install_linux.md",
    )
    if not host.which("gh"):
        return [Check("gh", MISSING, "GitHub CLI not found in PATH", [fix])]
    _, version = host.run(["gh", "--version"])
    checks = [Check("gh", OK, first_line(version))]
    code, status = host.run(["gh", "auth", "status"])
    if code == 0:
        checks.append(Check("gh auth", OK, "authenticated"))
    else:
        reason = next(
            (line.strip() for line in status.splitlines()
             if re.search(r"(?i:not logged|failed|invalid)|^\s*X ", line)),
            "not authenticated",
        )
        checks.append(Check("gh auth", MISSING, reason, ["gh auth login"]))
    return checks


def check_git(host: Host) -> List[Check]:
    fix = by_os(
        host.system,
        "winget install --id Git.Git -e",
        "xcode-select --install",
        "sudo apt install git",
    )
    if not host.which("git"):
        return [Check("git", MISSING, "git not found in PATH", [fix])]
    _, version = host.run(["git", "--version"])
    checks = [Check("git", OK, first_line(version))]
    for key in ("user.name", "user.email"):
        code, value = host.run(["git", "config", "--get", key])
        if code == 0 and value.strip():
            checks.append(Check(f"git {key}", OK, value.strip()))
        else:
            checks.append(Check(f"git {key}", MISSING, "not set", [f'git config --global {key} "<value>"']))
    return checks


def parse_requirements(text: str) -> List[Tuple[str, str]]:
    pins = []
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if "==" in line:
            name, version = line.split("==", 1)
            pins.append((name.strip(), version.strip()))
    return pins


def check_python(host: Host, repo: Path) -> List[Check]:
    command = python_command(host.system)
    shown = " ".join(command)
    fix = by_os(
        host.system,
        "winget install --id Python.Python.3.12 -e",
        "brew install python@3.12",
        "sudo apt install python3.12 python3.12-venv",
    )
    if not host.which(command[0]):
        return [Check("python 3.12", MISSING, f"{shown}: {command[0]} not found in PATH", [fix])]
    code, version = host.run(command + ["--version"])
    if code != 0 or not first_line(version).startswith("Python 3.12"):
        detail = first_line(version) or f"exit {code}"
        return [Check("python 3.12", MISSING, f"{shown}: {detail}", [fix])]
    checks = [Check("python 3.12", OK, f"{shown}: {first_line(version)}")]

    requirements = repo / "tests" / "ci" / "requirements.txt"
    pins = parse_requirements(host.read_text(requirements) or "")
    if pins:
        probe = (
            "import importlib.metadata as m,sys\n"
            "for n in sys.argv[1:]:\n"
            "    try: print(n, m.version(n))\n"
            "    except m.PackageNotFoundError: print(n, '-')\n"
        )
        _, out = host.run(command + ["-c", probe] + [name for name, _ in pins])
        installed = dict(line.split(" ", 1) for line in out.splitlines() if " " in line)
        wrong = [
            f"{name} {installed.get(name, '-')} (need {version})"
            for name, version in pins
            if installed.get(name) != version
        ]
        if wrong:
            if host.system == "Windows":
                fix = [f"{shown} -m pip install -r tests/ci/requirements.txt"]
            else:
                # Homebrew and Debian/Ubuntu Pythons refuse pip installs
                # (PEP 668); a venv keeps `python3.12` as its interpreter name.
                fix = [
                    'python3.12 -m venv "$HOME/.venvs/unica"',
                    '. "$HOME/.venvs/unica/bin/activate"',
                    "python3.12 -m pip install -r tests/ci/requirements.txt",
                ]
            checks.append(Check("python packages", MISSING, "; ".join(wrong), fix))
        else:
            checks.append(Check("python packages", OK, "tests/ci/requirements.txt satisfied"))
    return checks


def check_utf8(host: Host) -> Check:
    if host.system == "Windows":
        if host.env.get("PYTHONUTF8") == "1":
            return Check("PYTHONUTF8", OK, "PYTHONUTF8=1")
        return Check(
            "PYTHONUTF8", MISSING,
            "not set: Python reads repository files in the ANSI code page (cp1251 on ru-RU)",
            ["setx PYTHONUTF8 1   (then open a new shell and restart Claude/Codex)"],
        )
    encoding = host.preferred_encoding()
    if encoding.replace("-", "").lower() == "utf8":
        return Check("UTF-8 locale", OK, encoding)
    lang = "en_US.UTF-8" if host.system == "Darwin" else "C.UTF-8"
    return Check("UTF-8 locale", MISSING, f"locale encoding is {encoding}", [f"export LANG={lang}"])


def nextest_pin(host: Host, repo: Path) -> Optional[str]:
    workflow = host.read_text(repo / ".github" / "workflows" / "unica-plugin-release.yml") or ""
    match = re.search(r"cargo-nextest@([0-9][0-9.]*)", workflow)
    return match.group(1) if match else None


def check_rust(host: Host, repo: Path) -> List[Check]:
    rustup_fix = by_os(
        host.system,
        "winget install --id Rustlang.Rustup -e",
        "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh",
        "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh",
    )
    checks = []
    for tool in ("rustc", "cargo"):
        if not host.which(tool):
            checks.append(Check(tool, MISSING, f"{tool} not found in PATH", [rustup_fix]))
        else:
            _, version = host.run([tool, "--version"])
            checks.append(Check(tool, OK, first_line(version)))
    if any(check.status != OK for check in checks):
        return checks

    if host.which("rustup"):
        _, installed = host.run(["rustup", "component", "list", "--installed"])
        names = {TARGET_SUFFIX.sub("", line.strip()) for line in installed.splitlines()}
        absent = [component for component in RUST_COMPONENTS if component not in names]
        if absent:
            checks.append(Check(
                "rust components", MISSING, "missing: " + ", ".join(absent),
                ["rustup component add " + " ".join(RUST_COMPONENTS)],
            ))
        else:
            checks.append(Check("rust components", OK, ", ".join(RUST_COMPONENTS)))

    code, version = host.run(["cargo", "nextest", "--version"])
    if code == 0:
        checks.append(Check("cargo-nextest", OK, first_line(version)))
    else:
        pin = nextest_pin(host, repo)
        pinned = f" --version {pin}" if pin else ""
        checks.append(Check(
            "cargo-nextest", MISSING, "cargo nextest is not installed",
            [f"cargo install cargo-nextest{pinned} --locked"],
        ))
    return checks


def check_c_toolchain(host: Host) -> Check:
    if host.system == "Windows":
        roots = [host.env.get("PROGRAMFILES(X86)"), host.env.get("PROGRAMFILES")]
        fix = [
            "winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "
            '"--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"',
            f"needs about {MSVC_BUILD_TOOLS_GIB} GB",
        ]
        for root in roots:
            if not root:
                continue
            vswhere = Path(root) / VSWHERE
            if not host.is_file(vswhere):
                continue
            code, out = host.run([
                str(vswhere), "-latest", "-products", "*",
                "-requires", VC_TOOLS_COMPONENT, "-property", "installationPath",
            ])
            if code == 0 and first_line(out):
                return Check("MSVC Build Tools", OK, first_line(out))
        # Git for Windows ships an unrelated /usr/bin/link.exe; only the MSVC
        # linker counts.
        linker = host.which("link.exe") or ""
        if "Microsoft Visual Studio" in linker or "MSVC" in linker:
            return Check("MSVC Build Tools", OK, linker)
        return Check("MSVC Build Tools", MISSING, "MSVC C++ tools and Windows SDK not found", fix)
    if host.system == "Darwin":
        code, out = host.run(["xcode-select", "-p"])
        if code == 0:
            return Check("C toolchain", OK, first_line(out))
        return Check("C toolchain", MISSING, "Xcode Command Line Tools not found", ["xcode-select --install"])
    if host.which("cc"):
        return Check("C toolchain", OK, host.which("cc") or "cc")
    return Check("C toolchain", MISSING, "cc not found in PATH", ["sudo apt install build-essential"])


def default_build_root(host: Host) -> Path:
    """Where conveyor workers keep their Cargo build directories."""
    if host.system == "Windows":
        return Path(r"C:\cargo-target")
    return Path(host.env.get("HOME", "~")) / "cargo-target"


def check_dev_debug(host: Host) -> Check:
    value = host.env.get("CARGO_PROFILE_DEV_DEBUG")
    if value == DEV_DEBUG:
        return Check("dev debuginfo", OK, f"CARGO_PROFILE_DEV_DEBUG={DEV_DEBUG}", required=False)
    fix = by_os(
        host.system,
        f"setx CARGO_PROFILE_DEV_DEBUG {DEV_DEBUG}   (then open a new shell and restart Claude/Codex)",
        f"echo 'export CARGO_PROFILE_DEV_DEBUG={DEV_DEBUG}' >> \"$HOME/.zshrc\"",
        f"echo 'export CARGO_PROFILE_DEV_DEBUG={DEV_DEBUG}' >> \"$HOME/.bashrc\"",
    )
    shown = f"is {value}" if value else "is not set"
    return Check(
        "dev debuginfo", WARN,
        f"CARGO_PROFILE_DEV_DEBUG {shown}: full debug info makes each build directory several times larger",
        [fix], required=False,
    )


def check_build_dirs(host: Host, root: Path) -> Check:
    """Size of every build directory, so the largest stale one is easy to spot."""
    dirs = host.child_dirs(root)
    if not dirs:
        return Check("build dirs", OK, f"none under {root}", required=False)
    sizes = sorted(((host.dir_size(path), path.name) for path in dirs), reverse=True)
    total = sum(size for size, _ in sizes)
    listed = ", ".join(f"{name} {size / GIB:.1f} GB" for size, name in sizes)
    detail = f"{total / GIB:.1f} GB under {root}: {listed}"
    if host.free_bytes(root) >= LOW_FREE_GIB * GIB:
        return Check("build dirs", OK, detail, required=False)
    return Check(
        "build dirs", WARN, detail,
        ["py scripts/dev/prune-build-caches.py" if host.system == "Windows"
         else "python3.12 scripts/dev/prune-build-caches.py"],
        required=False,
    )


def check_capacity(host: Host, repo: Path, copies: int) -> Check:
    """How many working copies may build Rust at the same time."""
    cores = host.cpu_count()
    memory = host.memory_bytes()
    by_cores = max(1, cores // CORES_PER_BUILD)
    by_disk = max(0, host.free_bytes(repo) // (COPY_COMFORT_GIB * GIB))
    # The disk check reports a full disk; a copy that already exists can still build.
    builds = max(1, min(by_cores, by_disk))
    ram = f"{memory / GIB:.1f} GB RAM" if memory else "RAM unknown"
    detail = (
        f"{cores} logical cores, {ram}: at most {builds} parallel Rust "
        f"build{'' if builds == 1 else 's'} (1 per {CORES_PER_BUILD} cores: {by_cores}; "
        f"{COPY_COMFORT_GIB} GB free disk each: {by_disk})"
    )
    if copies <= builds:
        return Check("build capacity", OK, detail)
    return Check(
        "build capacity", WARN, detail + f"; {copies} planned",
        [f"keep at most {builds} working cop{'y' if builds == 1 else 'ies'} building at once"], required=False,
    )


def check_disk(host: Host, repo: Path, copies: int, msvc_missing: bool) -> Check:
    free = host.free_bytes(repo)
    need = COPY_COMFORT_GIB * copies + (MSVC_BUILD_TOOLS_GIB if msvc_missing else 0)
    minimum = COPY_MIN_GIB * copies + (MSVC_BUILD_TOOLS_GIB if msvc_missing else 0)
    detail = (
        f"{free / GIB:.1f} GB free at {repo}; {copies} working "
        f"cop{'y' if copies == 1 else 'ies'} with builds need {COPY_MIN_GIB * copies}-{COPY_COMFORT_GIB * copies} GB"
    )
    if msvc_missing:
        detail += f" plus about {MSVC_BUILD_TOOLS_GIB} GB for MSVC Build Tools"
    if free < LOW_FREE_GIB * GIB:
        detail += f"; below {LOW_FREE_GIB} GB a single test build can fill the disk"
    if free >= need * GIB and free >= LOW_FREE_GIB * GIB:
        return Check("disk", OK, detail)
    status = WARN if free >= minimum * GIB else MISSING
    return Check("disk", status, detail, ["free space or start fewer working copies"])


def pinned_platform_version(host: Host, repo: Path) -> Optional[str]:
    text = host.read_text(repo / "scripts" / "dev" / "verify-8-3-27-platform.py") or ""
    match = re.search(r'^EXPECTED_PLATFORM_VERSION\s*=\s*"([^"]+)"', text, re.M)
    return match.group(1) if match else None


def check_platform(host: Host, repo: Path, extra_roots: Sequence[str]) -> Check:
    executable = "ibcmd.exe" if host.system == "Windows" else "ibcmd"
    roots = list(extra_roots) + list(PLATFORM_ROOTS.get(host.system, PLATFORM_ROOTS["Linux"]))
    found: Dict[str, str] = {}
    for root in roots:
        for child in host.child_dirs(Path(root)):
            for candidate in (child / executable, child / "bin" / executable):
                if host.is_file(candidate):
                    found.setdefault(child.name, str(candidate))
    pinned = pinned_platform_version(host, repo)
    fix = [
        "install the 1C:Enterprise platform"
        + (f" {pinned}" if pinned else "")
        + " from releases.1c.ru (needed only for native platform checks)",
    ]
    if pinned and pinned in found:
        return Check("1C platform", OK, f"{pinned}: {found[pinned]}", required=False)
    if found:
        listed = ", ".join(sorted(found))
        detail = f"found {listed}" + (f"; pinned {pinned} is absent" if pinned else "")
        return Check("1C platform", WARN, detail, fix, required=False)
    return Check("1C platform", WARN, f"no {executable} under " + ", ".join(roots), fix, required=False)


def run_checks(
    host: Host, repo: Path, copies: int, platform_roots: Sequence[str], build_root: Optional[Path] = None,
) -> List[Check]:
    checks: List[Check] = []
    checks += check_git(host)
    checks += check_gh(host)
    checks += check_python(host, repo)
    checks.append(check_utf8(host))
    checks += check_rust(host, repo)
    c_toolchain = check_c_toolchain(host)
    checks.append(c_toolchain)
    msvc_missing = host.system == "Windows" and c_toolchain.status != OK
    checks.append(check_disk(host, repo, copies, msvc_missing))
    checks.append(check_capacity(host, repo, copies))
    checks.append(check_dev_debug(host))
    checks.append(check_build_dirs(host, build_root or default_build_root(host)))
    checks.append(check_platform(host, repo, platform_roots))
    return checks


def render(checks: Sequence[Check], system: str) -> str:
    lines = [f"Unica host preflight ({system})", ""]
    width = max(len(check.name) for check in checks)
    for check in checks:
        mark = {OK: "ok  ", WARN: "warn", MISSING: "MISS"}[check.status]
        lines.append(f"[{mark}] {check.name.ljust(width)}  {check.detail}")
        for fix in check.fix:
            lines.append(f"       {' ' * width}  -> {fix}")
    blocking = [check.name for check in checks if check.required and check.status == MISSING]
    lines.append("")
    lines.append("Not ready: " + ", ".join(blocking) if blocking else "Ready.")
    return "\n".join(lines)


def is_ready(checks: Sequence[Check]) -> bool:
    return not any(check.required and check.status == MISSING for check in checks)


def main(argv: Optional[Sequence[str]] = None, host: Optional[Host] = None, repo: Path = REPO_ROOT) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--copies", type=int, default=1, help="working copies with builds planned on this disk")
    parser.add_argument("--platform-root", action="append", default=[], help="extra 1C install root to search")
    parser.add_argument("--build-root", type=Path, help="directory holding the Cargo build directories")
    parser.add_argument("--json", action="store_true", help="print machine-readable results")
    args = parser.parse_args(argv)
    # Names, paths and tool output may be non-ASCII; a cp1251 or ASCII console
    # must not turn the report into a UnicodeEncodeError.
    reconfigure = getattr(sys.stdout, "reconfigure", None)
    if reconfigure:
        reconfigure(errors="backslashreplace")
    host = host or Host()
    checks = run_checks(host, repo, max(args.copies, 1), args.platform_root, args.build_root)
    if args.json:
        print(json.dumps({"system": host.system, "ready": is_ready(checks),
                          "checks": [asdict(check) for check in checks]}, indent=2))
    else:
        print(render(checks, host.system))
    return 0 if is_ready(checks) else 1


if __name__ == "__main__":
    sys.exit(main())
