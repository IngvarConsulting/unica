#!/usr/bin/env python3
"""Rebuild the platform XML corpus of export formats from the 2.20 seed.

For every export format the corpus holds the Designer export of an empty
configuration and an empty extension made by one build of the platform line
that writes that format:

    tests/fixtures/formats/platform_xml/<format>/
        empty_configuration/src/...      Designer export of the configuration
        empty_configuration/v8project.yaml
        empty_extension/ext/...          Designer export of the extension
        manifest.json                    build, steps and deviations

A fresh infobase gets new UUIDs on every creation, so the corpus is not made
from scratch. The seed is the 2.20 corpus. Through v8-runner it is built into
a .cf/.cfe package with ibcmd, loaded into a new file infobase of the target
build and exported back by Designer (`pull --force` with the `designer`
executor). The bytes of every exported file are written by the platform, the
UUIDs come from the seed, and a second run gives the same corpus byte for byte.

A build older than 8.3.27 neither reads 2.20 XML nor loads a package whose
compatibility mode is 8.3.27. For such a format the script lowers the
compatibility modes of the seed copy to the target line, builds the package
with the newest installed 8.3.27 and records the lowering in the manifest.
`ConfigDumpInfo.xml` is not kept: the 2.20 corpus has none, and the manifest
names the excluded files.

The newest installed build of the line is used unless `--build FORMAT=BUILD`
names another. Nothing is installed. Work happens in a temporary directory
(`--work` keeps it in a given place).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Sequence

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS_ROOT = REPO_ROOT / "tests" / "fixtures" / "formats" / "platform_xml"
SEED_FORMAT = "2.20"
SEED_LINE = "8.3.27"
EXCLUDED_FILES = ("ConfigDumpInfo.xml",)
# Loading or exporting an empty configuration takes seconds; a command still
# running after this waits on a dialog.
COMMAND_TIMEOUT_SECONDS = 600
COMPATIBILITY_PROPERTIES = (
    "CompatibilityMode",
    "ConfigurationExtensionCompatibilityMode",
)
ROOT_VERSION = re.compile(rb"<MetaDataObject\b[^>]*?\sversion=\"([^\"]*)\"", re.S)
EXTENSION_NAME = re.compile(rb"<Properties>.*?<Name>([^<]+)</Name>", re.S)


@dataclass(frozen=True)
class Target:
    line: str
    # Compatibility mode the seed copy is lowered to; None keeps the seed's.
    lowered_to: Optional[str] = None


TARGETS: Dict[str, Target] = {
    "2.17": Target("8.3.24", lowered_to="Version8_3_24"),
    "2.18": Target("8.3.25", lowered_to="Version8_3_25"),
    "2.19": Target("8.3.26", lowered_to="Version8_3_26"),
    "2.21": Target("8.5.1"),
    "2.22": Target("8.5.4"),
}


class CorpusError(RuntimeError):
    pass


def platform_roots() -> List[Path]:
    roots = []
    for variable in ("ProgramFiles", "ProgramFiles(x86)"):
        value = os.environ.get(variable)
        if value:
            roots.append(Path(value) / "1cv8")
    roots.append(Path("/opt/1cv8/x86_64"))
    return [root for root in roots if root.is_dir()]


def build_key(build: str) -> tuple:
    return tuple(int(part) for part in build.split("."))


def newest_build(line: str) -> str:
    pattern = re.compile(rf"^{re.escape(line)}\.\d+$")
    builds = {
        entry.name
        for root in platform_roots()
        for entry in root.iterdir()
        if entry.is_dir() and pattern.match(entry.name)
    }
    if not builds:
        raise CorpusError(f"no installed build of platform line {line}")
    return max(builds, key=build_key)


def run(command: Sequence[str], cwd: Optional[Path] = None) -> subprocess.CompletedProcess:
    try:
        return subprocess.run(
            list(command),
            cwd=cwd,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=COMMAND_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise CorpusError(f"{' '.join(command)}: {error}") from error


def find_ibcmd(build: str) -> Path:
    # The first platform root holding the build wins, in the order newest_build
    # reads them; on Linux ibcmd lies in the build directory itself.
    for root in platform_roots():
        for place in (root / build / "bin", root / build):
            for name in ("ibcmd.exe", "ibcmd"):
                if (place / name).is_file():
                    return place / name
    raise CorpusError(f"no ibcmd in platform build {build}")


def create_infobase(build: str, path: Path) -> None:
    """Create an empty file infobase with ibcmd of the build itself.

    `v8-runner infobase create` builds the infobase from the project's
    sources; here they are not exported yet, so the platform creates it.
    """
    done = run([str(find_ibcmd(build)), "infobase", "create", f"--db-path={path}"])
    if done.returncode != 0:
        raise CorpusError(f"ibcmd infobase create failed: {done.stderr[-400:]}")


class Runner:
    def __init__(self, executable: str) -> None:
        self.executable = executable

    def version(self) -> str:
        done = run([self.executable, "--version"])
        if done.returncode != 0 or not done.stdout.strip():
            raise CorpusError(f"v8-runner --version failed: {done.stderr[-400:]}")
        return done.stdout.strip()

    def __call__(self, project: Path, *args: str) -> dict:
        done = run([self.executable, "--json-message", "--no-color", *args], cwd=project)
        start = done.stdout.find("{")
        try:
            answer = json.loads(done.stdout[start:]) if start >= 0 else None
        except json.JSONDecodeError:
            answer = None
        if not isinstance(answer, dict):
            raise CorpusError(
                f"v8-runner {' '.join(args)} gave no JSON answer: "
                f"{done.stdout[-400:]}{done.stderr[-400:]}"
            )
        if not answer.get("ok"):
            message = (answer.get("error") or {}).get("message") or answer
            raise CorpusError(f"v8-runner {' '.join(args)} failed: {message}")
        return answer


def write_project(project: Path, build: str, sets: Sequence[tuple], local: str = "") -> None:
    lines = ["format: DESIGNER", "workPath: build", "source-set:"]
    for name, kind, path in sets:
        lines += [f"  - name: {name}", f"    type: {kind}", f"    path: {path}"]
    lines += ["tools:", "  platform:", f"    version: {build}", "    strict: true"]
    if local:
        lines += ["providers:", "  pull: designer"]
    project.mkdir(parents=True, exist_ok=True)
    (project / "v8project.yaml").write_text("\n".join(lines) + "\n", encoding="utf-8")
    if local:
        (project / "v8project.local.yaml").write_text(local, encoding="utf-8")


def lower_compatibility(root: Path, mode: str) -> List[str]:
    changed = []
    path = root / "Configuration.xml"
    data = path.read_bytes()
    for name in COMPATIBILITY_PROPERTIES:
        pattern = re.compile(rb"(<%s>)[^<]*(</%s>)" % (name.encode(), name.encode()))
        data, count = pattern.subn(rb"\g<1>" + mode.encode() + rb"\g<2>", data)
        if count:
            changed.append(name)
    if "ConfigurationExtensionCompatibilityMode" not in changed:
        raise CorpusError(f"{path}: no compatibility mode to lower")
    path.write_bytes(data)
    return changed


def copy_export(source: Path, destination: Path) -> None:
    for path in sorted(source.rglob("*")):
        relative = path.relative_to(source)
        if path.is_dir() or relative.as_posix() in EXCLUDED_FILES:
            continue
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(path.read_bytes())


def root_version(path: Path) -> Optional[str]:
    match = ROOT_VERSION.search(path.read_bytes())
    return match.group(1).decode() if match else None


def refresh(fmt: str, build: str, packager: str, runner: Runner, work: Path) -> dict:
    target = TARGETS[fmt]
    seed = CORPUS_ROOT / SEED_FORMAT
    seed_configuration = seed / "empty_configuration" / "src"
    seed_extension = seed / "empty_extension" / "ext"
    extension_match = EXTENSION_NAME.search((seed_extension / "Configuration.xml").read_bytes())
    if not extension_match:
        raise CorpusError("seed extension has no name")
    extension = extension_match.group(1).decode()

    package_project = work / fmt / "package"
    shutil.copytree(seed_configuration, package_project / "seed" / "main")
    shutil.copytree(seed_extension, package_project / "seed" / "ext")
    lowered: Dict[str, List[str]] = {}
    if target.lowered_to:
        lowered["empty_configuration"] = lower_compatibility(
            package_project / "seed" / "main", target.lowered_to
        )
        lowered["empty_extension"] = lower_compatibility(
            package_project / "seed" / "ext", target.lowered_to
        )
    write_project(
        package_project,
        packager,
        [("main", "CONFIGURATION", "seed/main"), ("ext", "EXTENSION", "seed/ext")],
    )
    runner(package_project, "convert", "main", "--to", "package")
    runner(package_project, "convert", "ext", "--to", "package")
    packages = package_project / "build" / "convert" / "out" / "packages"

    dump_project = work / fmt / "dump"
    infobase = dump_project / "ib"
    write_project(
        dump_project,
        build,
        [("main", "CONFIGURATION", "out/main"), (extension, "EXTENSION", "out/ext")],
        local=f"infobases:\n  origin:\n    connection: File=\"{infobase}\"\n",
    )
    (dump_project / "out" / "main").mkdir(parents=True)
    (dump_project / "out" / "ext").mkdir(parents=True)
    create_infobase(build, infobase)
    runner(dump_project, "upload", str(packages / "main.cf"))
    runner(dump_project, "upload", str(packages / "ext.cfe"), "--extension", extension)
    dumps = []
    for name in ("main", extension):
        answer = runner(dump_project, "pull", name, "--force")
        provider = ((answer.get("data") or {}).get("provider") or {}).get("selected")
        dumps.append(provider)
    if set(dumps) != {"designer"}:
        raise CorpusError(f"export was not made by Designer: {dumps}")

    excluded = sorted(
        f"{part}/{name}"
        for part, sub in (("empty_configuration/src", "main"), ("empty_extension/ext", "ext"))
        for name in EXCLUDED_FILES
        if (dump_project / "out" / sub / name).exists()
    )
    # The corpus is assembled aside and replaces the old one only when
    # complete, so a failure leaves the previous corpus as it was.
    staged = work / fmt / "corpus"
    copy_export(dump_project / "out" / "main", staged / "empty_configuration" / "src")
    shutil.copyfile(
        seed / "empty_configuration" / "v8project.yaml",
        staged / "empty_configuration" / "v8project.yaml",
    )
    copy_export(dump_project / "out" / "ext", staged / "empty_extension" / "ext")
    for part in ("empty_configuration/src", "empty_extension/ext"):
        found = root_version(staged / part / "Configuration.xml")
        if found != fmt:
            raise CorpusError(f"{fmt}/{part} was exported as format {found}")

    manifest = {
        "format": fmt,
        "build": build,
        "seed": SEED_FORMAT,
        "runner": runner.version(),
        "steps": [
            {"step": "package", "tool": "ibcmd (v8-runner convert --to package)", "build": packager},
            {"step": "create", "tool": "ibcmd infobase create", "build": build},
            {"step": "load", "tool": "designer (v8-runner upload)", "build": build},
            {"step": "export", "tool": "designer /DumpConfigToFiles (v8-runner pull --force)", "build": build},
        ],
        "compatibilityLowered": (
            {"to": target.lowered_to, "properties": lowered} if target.lowered_to else None
        ),
        "excludedFiles": excluded,
    }
    (staged / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    destination = CORPUS_ROOT / fmt
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(staged, destination)
    return manifest


def main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "formats", nargs="*", default=sorted(TARGETS), help="formats to refresh (default: all)"
    )
    parser.add_argument(
        "--build", action="append", default=[], metavar="FORMAT=BUILD",
        help="use this platform build for the format instead of the newest of its line",
    )
    parser.add_argument(
        "--runner", default=os.environ.get("V8_RUNNER", "v8-runner"),
        help="v8-runner executable (default: $V8_RUNNER or v8-runner from PATH)",
    )
    parser.add_argument("--work", type=Path, help="keep work files in this new directory")
    args = parser.parse_args(argv)

    unknown = [fmt for fmt in args.formats if fmt not in TARGETS]
    if unknown:
        parser.error(f"unknown format {unknown}; known: {sorted(TARGETS)}")
    builds = {}
    for item in args.build:
        fmt, _, build = item.partition("=")
        if fmt not in TARGETS or not re.fullmatch(r"\d+(\.\d+){3}", build):
            parser.error(f"--build expects FORMAT=BUILD with a known format: {item}")
        builds[fmt] = build
    if shutil.which(args.runner) is None and not Path(args.runner).is_file():
        parser.error(f"v8-runner not found: {args.runner}")
    runner = Runner(args.runner)

    if args.work:
        args.work = args.work.resolve()
        if args.work.exists():
            parser.error(f"--work must name a new directory: {args.work}")
        args.work.mkdir(parents=True)
    try:
        for fmt in args.formats:
            target = TARGETS[fmt]
            build = builds.get(fmt) or newest_build(target.line)
            packager = newest_build(SEED_LINE) if target.lowered_to else build
            work = args.work or Path(tempfile.mkdtemp(prefix=f"platform-corpus-{fmt}-"))
            try:
                refresh(fmt, build, packager, runner, work)
                print(f"{fmt}: build {build}, package by {packager}")
            finally:
                if not args.work:
                    shutil.rmtree(work, ignore_errors=True)
    except CorpusError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
