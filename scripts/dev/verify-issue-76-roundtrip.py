#!/usr/bin/env python3
"""Run the opt-in live round-trip reproducer for Unica issue #76.

The verifier never points Unica or 1C at the supplied inputs.  It first copies
the file infobase and the Designer source tree into a fresh private workspace,
then performs every mutation and runtime operation through the public Unica MCP
surface.  This is developer evidence, not a product or release entry point.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import queue
import re
import secrets
import shlex
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import xml.etree.ElementTree as ElementTree
from collections import deque
from pathlib import Path


SCENARIO = "issue-76-live-roundtrip"
SOURCE_SET = "main"
CATALOG_METADATA_PATH = "Catalog.ЗависимостиСчетов"
MODULE_METADATA_PATH = (
    "CommonModule.СообщенияВСлужбуТехническойПоддержкиБПКлиентСервер.Module"
)
CATALOG_RELATIVE_PATH = Path("src/Catalogs/ЗависимостиСчетов.xml")
COMMON_MODULE_DESCRIPTOR_RELATIVE_PATH = Path(
    "src/CommonModules/"
    "СообщенияВСлужбуТехническойПоддержкиБПКлиентСервер.xml"
)
MODULE_RELATIVE_PATH = Path(
    "src/CommonModules/"
    "СообщенияВСлужбуТехническойПоддержкиБПКлиентСервер/Ext/Module.bsl"
)
PARENT_CONFIGURATION_RELATIVE_PATH = Path(
    "src/Ext/ParentConfigurations/УправлениеХолдингом.cf"
)
CONFIG_DUMP_INFO_RELATIVE_PATH = Path("src/ConfigDumpInfo.xml")
MARKER_PREFIX = "UNICA_ISSUE_76_ROUND_TRIP"
MARKER_RE = re.compile(rf"{MARKER_PREFIX}(?:_[0-9A-F]{{32}})?\Z")
EDITABLE_SUPPORT_RULE_RECEIPTS = frozenset(
    {
        "editable",
        "редактируется с сохранением поддержки "
        "(объект продолжит получать обновления вендора — возможны конфликты при обновлении)",
    }
)
DEFAULT_TIMEOUT_SECONDS = 7200.0
MAX_TIMEOUT_SECONDS = 86400.0
DIAGNOSTIC_LIMIT = 4096
REQUIRED_TOOLS = frozenset(
    {
        "unica.cf.info",
        "unica.support.edit",
        "unica.meta.edit",
        "unica.code.patch",
        "unica.runtime.execute",
    }
)
PLATFORM_VERSION_RE = re.compile(r"8\.3\.27\.\d+\Z")
_CONNECTION_CREDENTIAL_RE = re.compile(
    r"(?i)(?<![A-Za-z0-9_])(?:Pwd|Password|Usr|User)\s*=\s*"
    r"(?:\"(?:[^\"]|\"\")*\"|[^;\s,}\]]*)"
)
_CLI_CREDENTIAL_RE = re.compile(
    r"(?i)--(?:user|username|connection|pwd|"
    r"[A-Za-z0-9_-]*(?:password|token|secret)[A-Za-z0-9_-]*)(?:=|\s+)"
    r"(?:\"[^\"]*\"|'[^']*'|\S+)"
)
_SECRET_ASSIGNMENT_RE = re.compile(
    r"(?ix)"
    r"(?<![A-Za-z0-9_-])"
    r"(?:[\"'](?:connection|pwd|[A-Za-z0-9_-]*(?:password|token|secret)"
    r"[A-Za-z0-9_-]*)[\"']|"
    r"(?:connection|pwd|[A-Za-z0-9_-]*(?:password|token|secret)"
    r"[A-Za-z0-9_-]*))"
    r"\s*(?:=|:)\s*"
    r"(?:\"(?:[^\"\\]|\\.)*\"|'(?:[^'\\]|\\.)*'|[^;,&\r\n}\s]+)"
)
_EXACT_SECRET_KEYS = frozenset(
    {"connection", "pwd", "user", "usr", "username", "db_user", "dbuser"}
)
_SUBSTRING_SECRET_KEYS = ("password", "token", "secret")
_MANIFEST_TOOL_NAMES = ("unica", "v8-runner")
_MANIFEST_TOOL_FIELDS = ("version", "sourceCommit", "sourceTag", "sha256")
_SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")


class SourceError(RuntimeError):
    """The live evidence could not be produced safely or completely."""


def _positive_timeout(value: str) -> float:
    try:
        parsed = float(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("must be a number") from error
    if (
        not math.isfinite(parsed)
        or parsed <= 0
        or parsed > MAX_TIMEOUT_SECONDS
    ):
        raise argparse.ArgumentTypeError(
            f"must be finite and in the range 0 < seconds <= {MAX_TIMEOUT_SECONDS:g}"
        )
    return parsed


def _platform_version(value: str) -> str:
    if PLATFORM_VERSION_RE.fullmatch(value) is None:
        raise argparse.ArgumentTypeError("must be an exact 8.3.27.x platform version")
    return value


def _argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Copy a file infobase and Designer source tree, mutate the copies "
            "through packaged Unica, build them, and verify a safe full dump."
        )
    )
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--binary-arg", action="append", default=[])
    parser.add_argument("--plugin-root", required=True, type=Path)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--sources", required=True, type=Path)
    parser.add_argument(
        "--parent-configuration",
        required=True,
        type=Path,
        help=(
            "exact УправлениеХолдингом.cf vendor payload copied only into the "
            "private source tree"
        ),
    )
    parser.add_argument("--platform-path", required=True, type=Path)
    parser.add_argument("--platform-version", required=True, type=_platform_version)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument(
        "--builder",
        choices=("DESIGNER", "IBCMD"),
        default="DESIGNER",
    )
    parser.add_argument("--db-user", default="Администратор")
    parser.add_argument(
        "--timeout-seconds",
        type=_positive_timeout,
        default=DEFAULT_TIMEOUT_SECONDS,
    )
    parser.add_argument(
        "--execute",
        action="store_true",
        required=True,
        help="explicitly opt in to running the mutating live scenario on copies",
    )
    parser.add_argument(
        "--allow-empty-password",
        action="store_true",
        required=True,
        help="explicitly confirm the selected copied infobase uses an empty password",
    )
    return parser


def _is_relative_to(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
    except ValueError:
        return False
    return True


def _paths_overlap(left: Path, right: Path) -> bool:
    return _is_relative_to(left, right) or _is_relative_to(right, left)


def _resolved_absolute(path: Path, label: str, *, directory: bool) -> Path:
    path = Path(path)
    if not path.is_absolute():
        raise SourceError(f"{label} must be an absolute path")
    if path.is_symlink():
        raise SourceError(f"{label} must not be a symlink")
    try:
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise SourceError(f"{label} does not exist: {error}") from error
    if directory and not resolved.is_dir():
        raise SourceError(f"{label} must be a directory")
    if not directory and not resolved.is_file():
        raise SourceError(f"{label} must be a regular file")
    return resolved


def _validate_report_path(
    report_path: Path,
    *,
    protected_paths: tuple[tuple[Path, str], ...],
) -> Path:
    raw = Path(report_path)
    if not raw.is_absolute():
        raise SourceError("report path must be absolute")
    if raw.is_symlink():
        raise SourceError("report path must not be a symlink")
    try:
        parent = raw.parent.resolve(strict=True)
    except OSError as error:
        raise SourceError(f"report parent directory does not exist: {error}") from error
    if not parent.is_dir():
        raise SourceError("report parent must be a directory")
    target = parent / raw.name
    if target.exists() and (target.is_symlink() or not target.is_file()):
        raise SourceError("report target exists and is not a safe regular file")
    for protected, label in protected_paths:
        if _paths_overlap(target, protected):
            raise SourceError(f"report path must be outside {label}")
    return target


def _validate_evidence_directory(
    evidence_dir: Path,
    *,
    database: Path,
    sources: Path,
    report_path: Path,
    protected_paths: tuple[tuple[Path, str], ...] = (),
) -> Path:
    evidence = _resolved_absolute(evidence_dir, "evidence directory", directory=True)
    repo = Path(__file__).resolve().parents[2]
    home = Path.home().resolve()
    filesystem_root = Path(evidence.anchor).resolve()
    if (
        evidence == filesystem_root
        or evidence == home
        or _paths_overlap(evidence, repo)
        or _paths_overlap(evidence, database)
        or _paths_overlap(evidence, sources)
        or any(_paths_overlap(evidence, path) for path, _label in protected_paths)
        or _is_relative_to(home, evidence)
        or _paths_overlap(evidence, report_path)
    ):
        raise SourceError(
            "evidence directory must be outside broad, home, repository, input, "
            "runtime, and report paths"
        )
    try:
        with os.scandir(evidence) as entries:
            if next(entries, None) is not None:
                raise SourceError("evidence directory must be empty")
    except OSError as error:
        raise SourceError(f"cannot inspect evidence directory: {error}") from error
    return evidence


def _safe_automatic_evidence_parent(
    *,
    protected_paths: tuple[tuple[Path, str], ...],
) -> Path:
    raw_candidates = [
        tempfile.tempdir,
        os.environ.get("TMPDIR"),
        os.environ.get("TEMP"),
        os.environ.get("TMP"),
        "/private/tmp",
        "/tmp",
        "/var/tmp",
        "/usr/tmp",
    ]
    seen = set()
    home = Path.home().resolve()
    repo = Path(__file__).resolve().parents[2]
    for raw in raw_candidates:
        if raw in (None, "", b""):
            continue
        try:
            candidate = Path(os.fsdecode(raw))
        except (TypeError, ValueError):
            continue
        if not candidate.is_absolute():
            continue
        try:
            candidate = candidate.resolve(strict=True)
        except OSError:
            continue
        if not candidate.is_dir() or candidate == Path(candidate.anchor):
            continue
        if _is_relative_to(candidate, home) or _is_relative_to(candidate, repo):
            continue
        if any(
            _is_relative_to(candidate, protected)
            for protected, _label in protected_paths
        ):
            continue
        if not os.access(candidate, os.W_OK | os.X_OK):
            continue
        seen_key = os.path.normcase(str(candidate))
        if seen_key in seen:
            continue
        seen.add(seen_key)
        return candidate
    raise SourceError(
        "no safe automatic evidence parent is available outside protected paths"
    )


def _require_regular_file(path: Path, label: str) -> Path:
    if path.is_symlink() or not path.is_file():
        raise SourceError(f"{label} must be a regular non-symlink file: {path}")
    return path


def _hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as error:
        raise SourceError(f"cannot hash regular file {path}: {error}") from error
    return digest.hexdigest()


def _copy_regular_file(source: Path, destination: Path) -> tuple[str, int]:
    try:
        before = source.lstat()
    except OSError as error:
        raise SourceError(f"cannot inspect input file {source}: {error}") from error
    if not stat.S_ISREG(before.st_mode):
        raise SourceError(f"input entry is not a regular file: {source}")
    if before.st_nlink != 1:
        raise SourceError(
            f"input file has a hardlink alias and cannot be copied safely: {source}"
        )
    source_flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    destination_flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    mode = 0o700 if before.st_mode & stat.S_IXUSR else 0o600
    digest = hashlib.sha256()
    total = 0
    try:
        source_fd = os.open(source, source_flags)
        try:
            opened = os.fstat(source_fd)
            if (
                not stat.S_ISREG(opened.st_mode)
                or opened.st_dev != before.st_dev
                or opened.st_ino != before.st_ino
            ):
                raise SourceError(f"input file changed before it could be copied: {source}")
            destination_fd = os.open(destination, destination_flags, mode)
            try:
                while True:
                    block = os.read(source_fd, 1024 * 1024)
                    if not block:
                        break
                    digest.update(block)
                    total += len(block)
                    view = memoryview(block)
                    while view:
                        written = os.write(destination_fd, view)
                        view = view[written:]
                os.fchmod(destination_fd, mode)
            finally:
                os.close(destination_fd)
            after = os.fstat(source_fd)
            if (
                after.st_dev != before.st_dev
                or after.st_ino != before.st_ino
                or after.st_size != before.st_size
                or after.st_mtime_ns != before.st_mtime_ns
            ):
                raise SourceError(f"input file changed while it was copied: {source}")
        finally:
            os.close(source_fd)
    except SourceError:
        raise
    except OSError as error:
        raise SourceError(f"cannot make private copy of {source}: {error}") from error
    return digest.hexdigest(), total


def _copy_regular_tree(source: Path, destination: Path) -> dict:
    """Copy a tree without links/special entries and return a compact receipt."""

    if destination.exists() or destination.is_symlink():
        raise SourceError(f"private copy target already exists: {destination}")
    try:
        destination.mkdir(mode=0o700)
    except OSError as error:
        raise SourceError(f"cannot create private copy target: {error}") from error

    receipt_digest = hashlib.sha256()
    file_count = 0
    directory_count = 1
    total_bytes = 0
    identities: set[tuple[int, int]] = set()

    def visit(source_dir: Path, destination_dir: Path, relative: Path) -> None:
        nonlocal file_count, directory_count, total_bytes
        try:
            with os.scandir(source_dir) as iterator:
                entries = sorted(iterator, key=lambda entry: entry.name)
        except OSError as error:
            raise SourceError(f"cannot enumerate input tree {source_dir}: {error}") from error
        if not entries:
            receipt_digest.update(b"D\0" + relative.as_posix().encode("utf-8") + b"\0")
        for entry in entries:
            source_path = Path(entry.path)
            destination_path = destination_dir / entry.name
            child_relative = relative / entry.name
            try:
                if entry.is_symlink():
                    raise SourceError(f"input symlink is forbidden: {source_path}")
                metadata = entry.stat(follow_symlinks=False)
                identity = (metadata.st_dev, metadata.st_ino)
                if identity in identities:
                    raise SourceError(
                        f"input filesystem identity is exposed more than once: {source_path}"
                    )
                identities.add(identity)
                if entry.is_dir(follow_symlinks=False):
                    destination_path.mkdir(mode=0o700)
                    directory_count += 1
                    receipt_digest.update(
                        b"D\0" + child_relative.as_posix().encode("utf-8") + b"\0"
                    )
                    visit(source_path, destination_path, child_relative)
                elif entry.is_file(follow_symlinks=False):
                    file_sha256, size = _copy_regular_file(source_path, destination_path)
                    file_count += 1
                    total_bytes += size
                    receipt_digest.update(
                        b"F\0"
                        + child_relative.as_posix().encode("utf-8")
                        + b"\0"
                        + file_sha256.encode("ascii")
                        + b"\0"
                    )
                else:
                    raise SourceError(f"special input entry is forbidden: {source_path}")
            except SourceError:
                raise
            except OSError as error:
                raise SourceError(f"cannot copy input entry {source_path}: {error}") from error

    visit(source, destination, Path())
    return {
        "sha256": receipt_digest.hexdigest(),
        "fileCount": file_count,
        "directoryCount": directory_count,
        "bytes": total_bytes,
    }


def _install_parent_configuration(parent_configuration: Path, source_copy: Path) -> dict:
    """Copy the explicit vendor payload into only one already-private source tree."""

    parent_configuration = _require_regular_file(
        Path(parent_configuration),
        "parent configuration payload",
    )
    source_copy = Path(source_copy)
    if source_copy.is_symlink() or not source_copy.is_dir():
        raise SourceError("private source copy must be a regular directory")
    destination = (
        source_copy / PARENT_CONFIGURATION_RELATIVE_PATH.relative_to("src")
    )
    if destination.exists() or destination.is_symlink():
        raise SourceError(
            "private source copy already contains the parent configuration payload; "
            "refusing to overwrite or merge it"
        )
    directory = destination.parent
    if directory.exists():
        if directory.is_symlink() or not directory.is_dir():
            raise SourceError(
                "private parent configuration destination must be a regular directory"
            )
    else:
        try:
            directory.mkdir(mode=0o700)
        except OSError as error:
            raise SourceError(
                f"cannot create private parent configuration directory: {error}"
            ) from error
    sha256, size = _copy_regular_file(parent_configuration, destination)
    return {
        "sha256": sha256,
        "bytes": size,
        "destination": "$EVIDENCE/workspace/"
        + PARENT_CONFIGURATION_RELATIVE_PATH.as_posix(),
    }


def _regular_file_stat_signature(path: Path) -> tuple[int, int, int, int, int, int]:
    path = _require_regular_file(path, "input file")
    try:
        metadata = path.stat()
    except OSError as error:
        raise SourceError(f"cannot inspect input file {path}: {error}") from error
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _stat_tree_digest(root: Path) -> str:
    """Bind input identities and timestamps without reading all bytes again."""

    digest = hashlib.sha256()
    identities: set[tuple[int, int]] = set()
    try:
        root_metadata = root.lstat()
    except OSError as error:
        raise SourceError(f"cannot inspect input state {root}: {error}") from error
    if not stat.S_ISDIR(root_metadata.st_mode):
        raise SourceError(f"input state root is not a directory: {root}")
    identities.add((root_metadata.st_dev, root_metadata.st_ino))
    digest.update(
        b"R\0.\0"
        + str(root_metadata.st_mode).encode("ascii")
        + b"\0"
        + str(root_metadata.st_size).encode("ascii")
        + b"\0"
        + str(root_metadata.st_mtime_ns).encode("ascii")
        + b"\0"
        + str(root_metadata.st_ctime_ns).encode("ascii")
        + b"\0"
        + str(root_metadata.st_dev).encode("ascii")
        + b"\0"
        + str(root_metadata.st_ino).encode("ascii")
        + b"\0"
    )

    def visit(directory: Path, relative: Path) -> None:
        try:
            with os.scandir(directory) as iterator:
                entries = sorted(iterator, key=lambda entry: entry.name)
        except OSError as error:
            raise SourceError(f"cannot enumerate input state {directory}: {error}") from error
        for entry in entries:
            path = Path(entry.path)
            if entry.is_symlink():
                raise SourceError(f"input symlink is forbidden: {path}")
            try:
                metadata = entry.stat(follow_symlinks=False)
            except OSError as error:
                raise SourceError(f"cannot inspect input state {path}: {error}") from error
            identity = (metadata.st_dev, metadata.st_ino)
            if identity in identities:
                raise SourceError(f"input hardlink alias is forbidden: {path}")
            identities.add(identity)
            child = relative / entry.name
            kind = b"D" if entry.is_dir(follow_symlinks=False) else b"F"
            if kind == b"F" and not entry.is_file(follow_symlinks=False):
                raise SourceError(f"special input entry is forbidden: {path}")
            record = (
                kind
                + b"\0"
                + child.as_posix().encode("utf-8")
                + b"\0"
                + str(metadata.st_mode).encode("ascii")
                + b"\0"
                + str(metadata.st_size).encode("ascii")
                + b"\0"
                + str(metadata.st_mtime_ns).encode("ascii")
                + b"\0"
                + str(metadata.st_ctime_ns).encode("ascii")
                + b"\0"
                + str(metadata.st_dev).encode("ascii")
                + b"\0"
                + str(metadata.st_ino).encode("ascii")
                + b"\0"
            )
            digest.update(record)
            if kind == b"D":
                visit(path, child)

    visit(root, Path())
    return digest.hexdigest()


def _write_new_file(path: Path, payload: bytes, *, mode: int) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    try:
        descriptor = os.open(path, flags, mode)
        try:
            view = memoryview(payload)
            while view:
                written = os.write(descriptor, view)
                view = view[written:]
            os.fchmod(descriptor, mode)
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    except OSError as error:
        raise SourceError(f"cannot create private project file {path}: {error}") from error


def _yaml_string(value: str) -> str:
    if not isinstance(value, str) or not value or "\0" in value:
        raise SourceError("project configuration strings must be non-empty and NUL-free")
    return json.dumps(value, ensure_ascii=False)


def _connection_component(value: str, label: str) -> str:
    if not isinstance(value, str) or not value or any(ord(char) < 32 for char in value):
        raise SourceError(f"{label} must be a non-empty string without control characters")
    return value.replace('"', '""')


def _write_project_configuration(
    workspace: Path,
    *,
    database_copy: Path,
    platform_path: Path,
    platform_version: str,
    db_user: str,
    builder: str,
    timeout_seconds: float,
) -> None:
    timeout_ms = max(1, int(round(timeout_seconds * 1000)))
    if timeout_ms > 86400000:
        raise SourceError("execution timeout exceeds the v8-runner 24-hour limit")
    primary = (
        f"execution_timeout: {timeout_ms}\n"
        "format: DESIGNER\n"
        f"builder: {builder}\n"
        "source-set:\n"
        f"  - name: {SOURCE_SET}\n"
        "    type: CONFIGURATION\n"
        "    path: src\n"
    ).encode("utf-8")
    database_value = _connection_component(str(database_copy), "database copy path")
    connection = f'File="{database_value}";'
    local = (
        f"workPath: {_yaml_string('work')}\n"
        "infobase:\n"
        f"  connection: {_yaml_string(connection)}\n"
        f"  user: {_yaml_string(db_user)}\n"
        '  password: ""\n'
        "tools:\n"
        "  platform:\n"
        f"    version: {_yaml_string(platform_version)}\n"
        f"    path: {_yaml_string(str(platform_path))}\n"
        "    strict: true\n"
    ).encode("utf-8")
    _write_new_file(workspace / "v8project.yaml", primary, mode=0o600)
    _write_new_file(workspace / "v8project.local.yaml", local, mode=0o600)


def _replace_project_platform_path(
    workspace: Path,
    *,
    previous_path: Path,
    next_path: Path,
) -> None:
    local_path = workspace / "v8project.local.yaml"
    _require_regular_file(local_path, "private project local configuration")
    try:
        preimage = local_path.read_bytes()
    except OSError as error:
        raise SourceError(f"cannot read private project local configuration: {error}") from error
    previous = f"    path: {_yaml_string(str(previous_path))}\n".encode("utf-8")
    replacement = f"    path: {_yaml_string(str(next_path))}\n".encode("utf-8")
    if preimage.count(previous) != 1:
        raise SourceError(
            "private project platform path changed before the full-dump phase"
        )
    payload = preimage.replace(previous, replacement, 1)
    temporary_path = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb",
            dir=local_path.parent,
            prefix=f".{local_path.name}.issue-76-platform.",
            delete=False,
        ) as stream:
            temporary_path = Path(stream.name)
            os.chmod(temporary_path, 0o600)
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        if local_path.read_bytes() != preimage:
            raise SourceError(
                "private project local configuration changed during platform switch"
            )
        os.replace(temporary_path, local_path)
        temporary_path = None
    except SourceError:
        raise
    except OSError as error:
        raise SourceError(
            f"cannot switch private project to the trusted full-dump platform: {error}"
        ) from error
    finally:
        if temporary_path is not None:
            try:
                temporary_path.unlink(missing_ok=True)
            except OSError:
                pass


def _create_private_ibcmd_platform(
    evidence: Path,
    *,
    trusted_platform: Path,
    platform_version: str,
) -> tuple[Path, dict]:
    if os.name != "posix":
        raise SourceError("private IBCMD data isolation is supported only on POSIX")
    trusted_ibcmd = _resolved_absolute(
        trusted_platform / "ibcmd",
        "trusted platform ibcmd",
        directory=False,
    )
    if not os.access(trusted_ibcmd, os.X_OK):
        raise SourceError("trusted platform ibcmd is not executable")
    if PLATFORM_VERSION_RE.fullmatch(platform_version) is None:
        raise SourceError("private IBCMD platform requires an exact 8.3.27.x version")
    wrapper_root = evidence / "ibcmd-platform"
    wrapper_platform = wrapper_root / platform_version
    private_data = evidence / "ibcmd-data"
    try:
        wrapper_root.mkdir(mode=0o700)
        wrapper_platform.mkdir(mode=0o700)
        private_data.mkdir(mode=0o700)
    except OSError as error:
        raise SourceError(f"cannot create private IBCMD isolation paths: {error}") from error
    wrapper = wrapper_platform / "ibcmd"
    payload = (
        "#!/bin/sh\n"
        "set -eu\n\n"
        'if [ "${1-}" = "infobase" ]; then\n'
        "  shift\n"
        '  for argument in "$@"; do\n'
        '    case "$argument" in\n'
        '      --data|--data=*) echo "refusing caller-supplied --data" >&2; exit 64 ;;\n'
        "    esac\n"
        "  done\n"
        f"  exec {shlex.quote(str(trusted_ibcmd))} infobase "
        f"--data {shlex.quote(str(private_data))} \"$@\"\n"
        "fi\n\n"
        f"exec {shlex.quote(str(trusted_ibcmd))} \"$@\"\n"
    ).encode("utf-8")
    _write_new_file(wrapper, payload, mode=0o700)
    return wrapper_platform, {
        "builder": "IBCMD",
        "privateIbcmdData": True,
        "buildPlatformPath": str(wrapper_platform),
        "buildDataPath": str(private_data),
        "fullDumpPlatformPath": str(trusted_platform),
        "wrapperSha256": hashlib.sha256(payload).hexdigest(),
        "trustedIbcmdSha256": _hash_file(trusted_ibcmd),
    }


def _redaction_pairs(redactions) -> list[tuple[str, str, bool]]:
    pairs: set[tuple[str, str, bool]] = set()
    for raw, replacement in redactions or []:
        value = str(raw)
        if not value:
            continue
        path_like = isinstance(raw, os.PathLike) or Path(value).is_absolute()
        pairs.add((value, str(replacement), path_like))
        if not path_like:
            continue
        try:
            resolved = str(Path(raw).resolve())
        except (OSError, TypeError, ValueError):
            continue
        if resolved:
            pairs.add((resolved, str(replacement), True))
    return sorted(pairs, key=lambda item: -len(item[0]))


def _sanitize_text(
    value: str,
    redactions,
    *,
    limit: int | None = None,
    redact_tokens: bool = True,
) -> str:
    text = value
    for raw, replacement, path_like in _redaction_pairs(redactions):
        if path_like:
            text = text.replace(raw, replacement)
        elif redact_tokens:
            token = re.compile(rf"(?<![\w$]){re.escape(raw)}(?![\w])")
            text = token.sub(lambda _match: replacement, text)
    text = _SECRET_ASSIGNMENT_RE.sub("<credential-redacted>", text)
    text = _CONNECTION_CREDENTIAL_RE.sub("<credential-redacted>", text)
    text = _CLI_CREDENTIAL_RE.sub("<credential-redacted>", text)
    if limit is not None and len(text) > limit:
        text = text[:limit] + "…<truncated>"
    return text


def _sanitize_key(value: str, redactions) -> str:
    text = value
    for raw, replacement, path_like in _redaction_pairs(redactions):
        if path_like:
            text = text.replace(raw, replacement)
    return text


def _is_secret_key(value: str) -> bool:
    normalized = value.casefold().lstrip("-")
    return normalized in _EXACT_SECRET_KEYS or any(
        marker in normalized for marker in _SUBSTRING_SECRET_KEYS
    )


def _sanitize_value(value, redactions, *, redact_tokens: bool = True):
    if isinstance(value, str):
        return _sanitize_text(value, redactions, redact_tokens=redact_tokens)
    if isinstance(value, Path):
        return _sanitize_text(
            str(value),
            redactions,
            redact_tokens=redact_tokens,
        )
    if isinstance(value, dict):
        sanitized = {}
        for key, child in value.items():
            key_text = str(key)
            sanitized_key = _sanitize_key(key_text, redactions)
            sanitized[sanitized_key] = (
                "<credential-redacted>"
                if _is_secret_key(key_text)
                else _sanitize_value(
                    child,
                    redactions,
                    redact_tokens=redact_tokens,
                )
            )
        return sanitized
    if isinstance(value, list):
        return [
            _sanitize_value(child, redactions, redact_tokens=redact_tokens)
            for child in value
        ]
    if isinstance(value, tuple):
        return [
            _sanitize_value(child, redactions, redact_tokens=redact_tokens)
            for child in value
        ]
    return value


def _json_digest(value) -> str:
    payload = json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        default=str,
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def _snapshot_tree(root: Path) -> dict:
    if root.is_symlink() or not root.is_dir():
        raise SourceError(f"source snapshot root is not a safe directory: {root}")
    files: dict[str, str] = {}
    empty_directories: list[str] = []
    identities: set[tuple[int, int]] = set()

    def visit(directory: Path) -> None:
        try:
            with os.scandir(directory) as iterator:
                entries = sorted(iterator, key=lambda entry: entry.name)
        except OSError as error:
            raise SourceError(f"cannot enumerate source snapshot {directory}: {error}") from error
        if not entries and directory != root:
            empty_directories.append(directory.relative_to(root).as_posix())
        for entry in entries:
            path = Path(entry.path)
            if entry.is_symlink():
                raise SourceError(f"source snapshot symlink is forbidden: {path}")
            try:
                metadata = entry.stat(follow_symlinks=False)
            except OSError as error:
                raise SourceError(f"cannot inspect source snapshot {path}: {error}") from error
            identity = (metadata.st_dev, metadata.st_ino)
            if identity in identities:
                raise SourceError(f"source snapshot hardlink alias is forbidden: {path}")
            identities.add(identity)
            if entry.is_dir(follow_symlinks=False):
                visit(path)
            elif entry.is_file(follow_symlinks=False):
                files[path.relative_to(root).as_posix()] = _hash_file(path)
            else:
                raise SourceError(f"source snapshot special entry is forbidden: {path}")

    visit(root)
    return {"files": files, "emptyDirectories": sorted(empty_directories)}


def _snapshot_digest(snapshot: dict) -> str:
    return _json_digest(snapshot)


def _optional_file_hash(path: Path) -> str | None:
    if not path.exists():
        return None
    _require_regular_file(path, "optional evidence file")
    return _hash_file(path)


def _restore_private_preimage(
    path: Path,
    payload: bytes,
    *,
    expected_current_sha256: str,
) -> None:
    """Atomically restore a private target after proving its mutation preimage."""

    _require_regular_file(path, "round-trip source target")
    if _hash_file(path) != expected_current_sha256:
        raise SourceError(
            "round-trip source target changed before the pre-dump oracle reset: "
            f"{path}"
        )
    temporary_path = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb",
            dir=path.parent,
            prefix=f".{path.name}.issue-76-reset.",
            delete=False,
        ) as stream:
            temporary_path = Path(stream.name)
            os.chmod(temporary_path, 0o600)
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_path, path)
    except OSError as error:
        if temporary_path is not None:
            try:
                temporary_path.unlink(missing_ok=True)
            except OSError:
                pass
        raise SourceError(f"cannot restore private round-trip preimage: {error}") from error


def _metadata_marker_survived(path: Path, marker: str) -> bool:
    try:
        root = ElementTree.fromstring(path.read_bytes())
    except (OSError, ElementTree.ParseError):
        return False
    comments = [
        element
        for element in root.iter()
        if element.tag.rsplit("}", 1)[-1] == "Comment"
    ]
    return len(comments) == 1 and (comments[0].text or "") == marker


def _metadata_marker_present(path: Path, marker: str) -> bool:
    try:
        root = ElementTree.fromstring(path.read_bytes())
    except (OSError, ElementTree.ParseError):
        return False
    return any(
        element.tag.rsplit("}", 1)[-1] == "Comment"
        and (element.text or "") == marker
        for element in root.iter()
    )


def _metadata_issue_marker_present(path: Path) -> bool:
    try:
        root = ElementTree.fromstring(path.read_bytes())
    except (OSError, ElementTree.ParseError):
        return False
    return any(
        element.tag.rsplit("}", 1)[-1] == "Comment"
        and (element.text or "").startswith(MARKER_PREFIX)
        for element in root.iter()
    )


def _module_marker_survived(path: Path, marker: str) -> bool:
    try:
        text = path.read_bytes().decode("utf-8-sig")
    except (OSError, UnicodeDecodeError):
        return False
    marker_index = text.find(marker)
    method_index = text.find("ЛинияПоддержки")
    return (
        marker_index >= 0
        and text.count(marker) == 1
        and method_index > marker_index
    )


def _module_marker_present(path: Path, marker: str) -> bool:
    try:
        return marker in path.read_bytes().decode("utf-8-sig")
    except (OSError, UnicodeDecodeError):
        return False


def _new_scenario_marker() -> str:
    return f"{MARKER_PREFIX}_{secrets.token_hex(16).upper()}"


def _scenario_markers(marker: str | None) -> tuple[str, str]:
    selected = _new_scenario_marker() if marker is None else marker
    if not isinstance(selected, str) or MARKER_RE.fullmatch(selected) is None:
        raise SourceError("scenario marker has an invalid format")
    return selected, f"// {selected}"


def _text_profile(path: Path) -> dict | None:
    try:
        payload = path.read_bytes()
    except OSError:
        return None
    crlf = payload.count(b"\r\n")
    bare_lf = payload.count(b"\n") - crlf
    return {
        "bomPrefixBytes": 3 if payload.startswith(b"\xef\xbb\xbf") else 0,
        "crlfCount": crlf,
        "bareLfCount": bare_lf,
        "terminalNewline": payload.endswith((b"\r", b"\n")),
    }


def _step_record(
    *,
    step_id: str,
    tool: str,
    arguments: dict,
    payload: dict,
    duration_ms: int,
    redactions,
) -> dict:
    sanitized_arguments = _sanitize_value(arguments, redactions)
    sanitized_payload = _sanitize_value(payload, redactions)
    projection = {
        "ok": payload.get("ok"),
        "summary": _sanitize_text(
            str(payload.get("summary", "")), redactions, limit=DIAGNOSTIC_LIMIT
        ),
        "warnings": [
            _sanitize_text(str(item), redactions, limit=DIAGNOSTIC_LIMIT)
            for item in payload.get("warnings", [])
        ],
        "errors": [
            _sanitize_text(str(item), redactions, limit=DIAGNOSTIC_LIMIT)
            for item in payload.get("errors", [])
        ],
    }
    for stream in ("stdout", "stderr"):
        if payload.get(stream) not in (None, ""):
            projection[stream] = _sanitize_text(
                str(payload[stream]), redactions, limit=DIAGNOSTIC_LIMIT
            )
    return {
        "id": step_id,
        "tool": tool,
        "arguments": sanitized_arguments,
        "argumentsSha256": _json_digest(sanitized_arguments),
        "resultSha256": _json_digest(sanitized_payload),
        "result": projection,
        "durationMs": duration_ms,
    }


def _invoke_step(client, report: dict, step_id: str, tool: str, arguments: dict, redactions):
    started = time.monotonic()
    try:
        payload = client.call(tool, arguments)
    except SourceError:
        raise
    except Exception as error:
        raise SourceError(f"{tool} call failed at {step_id}: {error}") from error
    duration_ms = int(round((time.monotonic() - started) * 1000))
    if not isinstance(payload, dict):
        raise SourceError(f"{tool} returned a non-object payload at {step_id}")
    report["steps"].append(
        _step_record(
            step_id=step_id,
            tool=tool,
            arguments=arguments,
            payload=payload,
            duration_ms=duration_ms,
            redactions=redactions,
        )
    )
    return payload


def _new_flow_report(
    workspace: Path,
    *,
    metadata_marker: str,
    bsl_marker: str,
) -> dict:
    catalog = workspace / CATALOG_RELATIVE_PATH
    module = workspace / MODULE_RELATIVE_PATH
    return {
        "schemaVersion": 1,
        "scenario": SCENARIO,
        "status": "failed",
        "exitCode": 1,
        "steps": [],
        "builds": {
            "baselineBuild": {"stepId": "baseline-build", "ok": False},
            "mutationBuild": {"stepId": "mutation-build", "ok": False},
        },
        "partialGuard": {
            "blocked": False,
            "sourceUnchanged": False,
            "beforeSha256": None,
            "afterSha256": None,
        },
        "roundTrip": {
            "metadata": {
                "metadataPath": CATALOG_METADATA_PATH,
                "marker": metadata_marker,
                "beforeSha256": _optional_file_hash(catalog),
                "afterMutationSha256": None,
                "beforeFullDumpSha256": None,
                "presentAfterMutation": False,
                "absentBeforeFullDump": False,
                "afterFullDumpSha256": None,
                "survived": False,
            },
            "module": {
                "metadataPath": MODULE_METADATA_PATH,
                "marker": bsl_marker,
                "beforeSha256": _optional_file_hash(module),
                "afterMutationSha256": None,
                "beforeFullDumpSha256": None,
                "presentAfterMutation": False,
                "absentBeforeFullDump": False,
                "afterFullDumpSha256": None,
                "survived": False,
                "textProfile": None,
            },
        },
        "configDumpInfo": {
            "before": _optional_file_hash(workspace / CONFIG_DUMP_INFO_RELATIVE_PATH),
            "afterBaselineBuild": None,
            "afterBuild": None,
            "afterFullDump": None,
            "changedByBaselineBuild": None,
            "changedByBuild": None,
            "changedByFullDump": None,
            "informationalOnly": True,
        },
        "summary": {"failures": []},
    }


def _finish_flow(report: dict, status: str, exit_code: int, redactions, failure=None):
    if failure:
        report["summary"]["failures"].append(str(failure))
    report["status"] = status
    report["exitCode"] = exit_code
    report["summary"]["stepCount"] = len(report["steps"])
    report["summary"]["passed"] = status == "pass"
    return exit_code, _sanitize_value(report, redactions, redact_tokens=False)


def _payload_ok(payload: dict) -> bool:
    return payload.get("ok") is True


def _support_state_matches(payload: dict, *, editing_enabled: bool) -> bool:
    data = payload.get("data")
    support = data.get("support") if isinstance(data, dict) else None
    return (
        isinstance(support, dict)
        and support.get("state") == "supported"
        and support.get("editingEnabled") is editing_enabled
    )


def _support_apply_matches(payload: dict, *, action: str) -> bool:
    data = payload.get("data")
    if not isinstance(data, dict):
        return False
    if data.get("action") != action or data.get("applied") is not True:
        return False
    if action == "capability":
        return data.get("editingEnabled") is True
    records_changed = data.get("recordsChanged")
    return (
        action == "objectRule"
        and isinstance(records_changed, int)
        and not isinstance(records_changed, bool)
        and records_changed > 0
        and data.get("rule") in EDITABLE_SUPPORT_RULE_RECEIPTS
    )


def _guard_is_blocked(payload: dict) -> bool:
    if payload.get("ok") is not False:
        return False
    combined = "\n".join(
        [str(payload.get("summary", ""))]
        + [str(item) for item in payload.get("errors", [])]
    ).casefold()
    return (
        "source sync guard" in combined
        and "v8-runner-rust#30" in combined
        and "divergence-safe merge" in combined
    )


def run_roundtrip_flow(
    client,
    *,
    workspace: Path,
    redactions=None,
    marker: str | None = None,
    before_full_dump=None,
) -> tuple[int, dict]:
    """Run issue #76 against one already-private workspace.

    ``client`` is the intentionally small test seam: it provides
    ``call(tool_name, arguments) -> dict``.  Production supplies the MCP stdio
    client below; unit tests supply a scripted public-tool fake.
    """

    workspace = Path(workspace)
    source = workspace / "src"
    catalog = workspace / CATALOG_RELATIVE_PATH
    common_module_descriptor = workspace / COMMON_MODULE_DESCRIPTOR_RELATIVE_PATH
    module = workspace / MODULE_RELATIVE_PATH
    for path, label in (
        (source / "Configuration.xml", "configuration descriptor"),
        (catalog, "catalog descriptor"),
        (common_module_descriptor, "common module descriptor"),
        (module, "common module source"),
    ):
        _require_regular_file(path, label)

    redactions = list(redactions or [])
    metadata_marker, bsl_marker = _scenario_markers(marker)
    report = _new_flow_report(
        workspace,
        metadata_marker=metadata_marker,
        bsl_marker=bsl_marker,
    )
    try:
        catalog_preimage = catalog.read_bytes()
        module_preimage = module.read_bytes()
    except OSError as error:
        raise SourceError(f"cannot read round-trip source preimages: {error}") from error

    preexisting_markers = []
    if _metadata_issue_marker_present(catalog):
        preexisting_markers.append("metadata")
    if _module_marker_present(module, f"// {MARKER_PREFIX}"):
        preexisting_markers.append("module")
    if preexisting_markers:
        return _finish_flow(
            report,
            "failed",
            1,
            redactions,
            "scenario marker already present before baseline build: "
            + ", ".join(preexisting_markers),
        )

    def invoke(step_id: str, tool: str, arguments: dict) -> dict:
        return _invoke_step(client, report, step_id, tool, arguments, redactions)

    cf_before = invoke(
        "support-info-before",
        "unica.cf.info",
        {"sourceSet": SOURCE_SET},
    )
    if not _payload_ok(cf_before):
        return _finish_flow(report, "failed", 1, redactions, "initial cf.info failed")
    if not _support_state_matches(cf_before, editing_enabled=False):
        return _finish_flow(
            report,
            "failed",
            1,
            redactions,
            "support precondition is not a supported configuration with editing disabled",
        )

    support_operations = [
        (
            "support-capability",
            {"Path": "src", "Capability": "on"},
        ),
        (
            "support-catalog",
            {"Path": CATALOG_RELATIVE_PATH.as_posix(), "Set": "editable"},
        ),
        (
            "support-common-module",
            {
                "Path": COMMON_MODULE_DESCRIPTOR_RELATIVE_PATH.as_posix(),
                "Set": "editable",
            },
        ),
    ]
    for step_id, operation in support_operations:
        for dry_run, suffix in ((True, "preview"), (False, "apply")):
            arguments = {
                "cwd": str(workspace),
                **operation,
                "dryRun": dry_run,
            }
            payload = invoke(
                f"{step_id}-{suffix}",
                "unica.support.edit",
                arguments,
            )
            if not _payload_ok(payload):
                return _finish_flow(
                    report,
                    "failed",
                    1,
                    redactions,
                    f"{step_id} {suffix} failed",
                )
            if not dry_run:
                expected_action = (
                    "capability" if step_id == "support-capability" else "objectRule"
                )
                if not _support_apply_matches(payload, action=expected_action):
                    return _finish_flow(
                        report,
                        "failed",
                        1,
                        redactions,
                        f"{step_id} apply did not report the required support transition",
                    )

    cf_after = invoke(
        "support-info-after",
        "unica.cf.info",
        {"sourceSet": SOURCE_SET},
    )
    if not _payload_ok(cf_after):
        return _finish_flow(report, "failed", 1, redactions, "final cf.info failed")
    if not _support_state_matches(cf_after, editing_enabled=True):
        return _finish_flow(
            report,
            "failed",
            1,
            redactions,
            "support postcondition did not confirm enabled configuration editing",
        )

    baseline_build = invoke(
        "baseline-build",
        "unica.runtime.execute",
        {
            "cwd": str(workspace),
            "operation": "build",
            "sourceSet": SOURCE_SET,
            "fullRebuild": True,
            "dryRun": False,
        },
    )
    report["builds"]["baselineBuild"]["ok"] = _payload_ok(baseline_build)
    if not _payload_ok(baseline_build):
        return _finish_flow(
            report,
            "failed",
            1,
            redactions,
            "support baseline build failed",
        )
    cdfi = workspace / CONFIG_DUMP_INFO_RELATIVE_PATH
    report["configDumpInfo"]["afterBaselineBuild"] = _optional_file_hash(cdfi)
    report["configDumpInfo"]["changedByBaselineBuild"] = (
        report["configDumpInfo"]["afterBaselineBuild"]
        != report["configDumpInfo"]["before"]
    )

    meta_arguments = {
        "sourceSet": SOURCE_SET,
        "metadataPath": CATALOG_METADATA_PATH,
        "operations": [
            {
                "op": "setProperties",
                "values": {"Comment": metadata_marker},
            }
        ],
    }
    for dry_run, suffix in ((True, "preview"), (False, "apply")):
        payload = invoke(
            f"meta-edit-{suffix}",
            "unica.meta.edit",
            {**meta_arguments, "dryRun": dry_run},
        )
        if not _payload_ok(payload):
            return _finish_flow(
                report, "failed", 1, redactions, f"meta.edit {suffix} failed"
            )

    code_arguments = {
        "cwd": str(workspace),
        "sourceSet": SOURCE_SET,
        "metadataPath": MODULE_METADATA_PATH,
        "operation": "insert",
        "selector": {"method": "ЛинияПоддержки"},
        "position": "before",
        "content": bsl_marker,
    }
    for dry_run, suffix in ((True, "preview"), (False, "apply")):
        payload = invoke(
            f"code-patch-{suffix}",
            "unica.code.patch",
            {**code_arguments, "dryRun": dry_run},
        )
        if not _payload_ok(payload):
            return _finish_flow(
                report, "failed", 1, redactions, f"code.patch {suffix} failed"
            )

    report["roundTrip"]["metadata"]["afterMutationSha256"] = _hash_file(catalog)
    report["roundTrip"]["module"]["afterMutationSha256"] = _hash_file(module)
    report["roundTrip"]["metadata"]["presentAfterMutation"] = (
        _metadata_marker_survived(catalog, metadata_marker)
    )
    report["roundTrip"]["module"]["presentAfterMutation"] = (
        _module_marker_survived(module, bsl_marker)
    )
    if not all(
        (
            report["roundTrip"]["metadata"]["presentAfterMutation"],
            report["roundTrip"]["module"]["presentAfterMutation"],
        )
    ):
        return _finish_flow(
            report,
            "failed",
            1,
            redactions,
            "one or both source mutations were not observable before runtime",
        )

    before_partial = _snapshot_tree(source)
    partial = invoke(
        "partial-dump-guard",
        "unica.runtime.execute",
        {
            "cwd": str(workspace),
            "operation": "dump",
            "mode": "partial",
            "object": "Catalog:ЗависимостиСчетов",
            "sourceSet": SOURCE_SET,
            "dryRun": False,
        },
    )
    after_partial = _snapshot_tree(source)
    report["partialGuard"].update(
        {
            "blocked": _guard_is_blocked(partial),
            "sourceUnchanged": before_partial == after_partial,
            "beforeSha256": _snapshot_digest(before_partial),
            "afterSha256": _snapshot_digest(after_partial),
        }
    )
    if not report["partialGuard"]["blocked"]:
        return _finish_flow(
            report,
            "failed",
            1,
            redactions,
            "applied partial dump was not rejected by the issue #76 source sync guard",
        )
    if not report["partialGuard"]["sourceUnchanged"]:
        return _finish_flow(
            report,
            "failed",
            1,
            redactions,
            "the rejected applied partial dump changed the private source tree",
        )

    build = invoke(
        "mutation-build",
        "unica.runtime.execute",
        {
            "cwd": str(workspace),
            "operation": "build",
            "sourceSet": SOURCE_SET,
            "dryRun": False,
        },
    )
    report["builds"]["mutationBuild"]["ok"] = _payload_ok(build)
    if not _payload_ok(build):
        return _finish_flow(report, "failed", 1, redactions, "mutation build failed")

    report["configDumpInfo"]["afterBuild"] = _optional_file_hash(cdfi)
    report["configDumpInfo"]["changedByBuild"] = (
        report["configDumpInfo"]["afterBuild"]
        != report["configDumpInfo"]["afterBaselineBuild"]
    )

    if before_full_dump is not None:
        before_full_dump()

    _restore_private_preimage(
        catalog,
        catalog_preimage,
        expected_current_sha256=report["roundTrip"]["metadata"][
            "afterMutationSha256"
        ],
    )
    _restore_private_preimage(
        module,
        module_preimage,
        expected_current_sha256=report["roundTrip"]["module"][
            "afterMutationSha256"
        ],
    )
    report["roundTrip"]["metadata"]["beforeFullDumpSha256"] = _hash_file(catalog)
    report["roundTrip"]["module"]["beforeFullDumpSha256"] = _hash_file(module)
    report["roundTrip"]["metadata"]["absentBeforeFullDump"] = not (
        _metadata_marker_present(catalog, metadata_marker)
    )
    report["roundTrip"]["module"]["absentBeforeFullDump"] = not (
        _module_marker_present(module, bsl_marker)
    )
    if not all(
        (
            report["roundTrip"]["metadata"]["absentBeforeFullDump"],
            report["roundTrip"]["module"]["absentBeforeFullDump"],
            report["roundTrip"]["metadata"]["beforeFullDumpSha256"]
            == report["roundTrip"]["metadata"]["beforeSha256"],
            report["roundTrip"]["module"]["beforeFullDumpSha256"]
            == report["roundTrip"]["module"]["beforeSha256"],
        )
    ):
        return _finish_flow(
            report,
            "failed",
            1,
            redactions,
            "could not establish marker-free source preimages before the full dump",
        )

    full_dump = invoke(
        "safe-full-dump",
        "unica.runtime.execute",
        {
            "cwd": str(workspace),
            "operation": "dump",
            "mode": "full",
            "sourceSet": SOURCE_SET,
            "dryRun": False,
        },
    )
    if not _payload_ok(full_dump):
        return _finish_flow(report, "failed", 1, redactions, "safe full dump failed")

    report["configDumpInfo"]["afterFullDump"] = _optional_file_hash(cdfi)
    report["configDumpInfo"]["changedByFullDump"] = (
        report["configDumpInfo"]["afterFullDump"]
        != report["configDumpInfo"]["afterBuild"]
    )
    report["roundTrip"]["metadata"]["afterFullDumpSha256"] = _optional_file_hash(
        catalog
    )
    report["roundTrip"]["module"]["afterFullDumpSha256"] = _optional_file_hash(module)
    report["roundTrip"]["metadata"]["survived"] = _metadata_marker_survived(
        catalog,
        metadata_marker,
    )
    report["roundTrip"]["module"]["survived"] = _module_marker_survived(
        module,
        bsl_marker,
    )
    report["roundTrip"]["module"]["textProfile"] = _text_profile(module)

    lost = [
        name
        for name in ("metadata", "module")
        if not report["roundTrip"][name]["survived"]
    ]
    if lost:
        return _finish_flow(
            report,
            "failed",
            1,
            redactions,
            "marker lost after safe full dump: " + ", ".join(lost),
        )
    return _finish_flow(report, "pass", 0, redactions)


class McpSession:
    """Sequential JSONL MCP client with bounded stderr and request deadlines."""

    def __init__(
        self,
        command: list[str],
        environment: dict[str, str],
        timeout_seconds: float,
        *,
        cwd: Path,
    ) -> None:
        if not command or any(not isinstance(item, str) or not item for item in command):
            raise SourceError("Unica command must be a non-empty argument array")
        self.timeout_seconds = timeout_seconds
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.diagnostics: deque[str] = deque(maxlen=256)
        self.next_id = 1
        popen_options = {}
        if os.name == "posix":
            popen_options["start_new_session"] = True
        elif os.name == "nt" and hasattr(subprocess, "CREATE_NEW_PROCESS_GROUP"):
            popen_options["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
        try:
            self.process = subprocess.Popen(
                command,
                cwd=cwd,
                env=environment,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
                **popen_options,
            )
        except (OSError, ValueError) as error:
            raise SourceError(f"cannot start packaged Unica MCP: {error}") from error
        self.stdout_reader = threading.Thread(target=self._read_stdout, daemon=True)
        self.stderr_reader = threading.Thread(target=self._read_stderr, daemon=True)
        self.stdout_reader.start()
        self.stderr_reader.start()

    def _read_stdout(self) -> None:
        assert self.process.stdout is not None
        try:
            for line in self.process.stdout:
                self.lines.put(line)
        finally:
            self.lines.put(None)

    def _read_stderr(self) -> None:
        assert self.process.stderr is not None
        for line in self.process.stderr:
            self.diagnostics.append(line[-DIAGNOSTIC_LIMIT:])

    def _diagnostic_text(self) -> str:
        return "".join(self.diagnostics)[-DIAGNOSTIC_LIMIT:].strip() or "no process output"

    def _terminate(self) -> None:
        if self.process.poll() is not None:
            return
        try:
            if os.name == "posix":
                os.killpg(self.process.pid, signal.SIGTERM)
            else:
                self.process.terminate()
        except (OSError, ProcessLookupError):
            pass
        try:
            self.process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            try:
                if os.name == "posix":
                    os.killpg(self.process.pid, signal.SIGKILL)
                else:
                    self.process.kill()
            except (OSError, ProcessLookupError):
                pass
            self.process.wait(timeout=2)

    def request(self, message: dict) -> dict:
        if self.process.poll() is not None:
            raise SourceError(
                f"packaged Unica exited before request: {self._diagnostic_text()}"
            )
        assert self.process.stdin is not None
        try:
            self.process.stdin.write(
                json.dumps(message, ensure_ascii=False, separators=(",", ":")) + "\n"
            )
            self.process.stdin.flush()
        except (OSError, BrokenPipeError) as error:
            self._terminate()
            raise SourceError(f"cannot write packaged Unica MCP request: {error}") from error
        deadline = time.monotonic() + self.timeout_seconds
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self._terminate()
                raise SourceError(
                    f"packaged Unica MCP request timed out after "
                    f"{self.timeout_seconds:g}s: {self._diagnostic_text()}"
                )
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty as error:
                self._terminate()
                raise SourceError(
                    f"packaged Unica MCP request timed out after "
                    f"{self.timeout_seconds:g}s: {self._diagnostic_text()}"
                ) from error
            if line is None:
                raise SourceError(
                    f"packaged Unica exited before the expected MCP response: "
                    f"{self._diagnostic_text()}"
                )
            try:
                response = json.loads(line)
            except json.JSONDecodeError as error:
                self._terminate()
                raise SourceError(f"packaged Unica emitted invalid JSON: {error}") from error
            if isinstance(response, dict) and response.get("id") == message.get("id"):
                return response

    def notify(self, message: dict) -> None:
        assert self.process.stdin is not None
        try:
            self.process.stdin.write(
                json.dumps(message, ensure_ascii=False, separators=(",", ":")) + "\n"
            )
            self.process.stdin.flush()
        except (OSError, BrokenPipeError) as error:
            self._terminate()
            raise SourceError(f"cannot write packaged Unica MCP notification: {error}") from error

    def start(self, required_tools=REQUIRED_TOOLS) -> None:
        initialize = self.request(
            {
                "jsonrpc": "2.0",
                "id": self.next_id,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "unica-issue-76-roundtrip",
                        "version": "1",
                    },
                },
            }
        )
        self.next_id += 1
        if "result" not in initialize:
            raise SourceError(f"packaged Unica initialize failed: {initialize.get('error')}")
        self.notify(
            {
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {},
            }
        )
        listed = self.request(
            {
                "jsonrpc": "2.0",
                "id": self.next_id,
                "method": "tools/list",
                "params": {},
            }
        )
        self.next_id += 1
        try:
            names = {
                item["name"]
                for item in listed["result"]["tools"]
                if isinstance(item, dict) and isinstance(item.get("name"), str)
            }
        except (KeyError, TypeError) as error:
            raise SourceError("packaged Unica tools/list response is malformed") from error
        missing = sorted(set(required_tools) - names)
        if missing:
            raise SourceError("packaged Unica is missing required tools: " + ", ".join(missing))

    def call(self, name: str, arguments: dict, **_kwargs) -> dict:
        response = self.request(
            {
                "jsonrpc": "2.0",
                "id": self.next_id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            }
        )
        self.next_id += 1
        if "error" in response:
            error = response["error"]
            raise SourceError(f"{name} failed as JSON-RPC: {error.get('message', error)}")
        try:
            result = response["result"]
        except (KeyError, TypeError) as error:
            raise SourceError(f"{name} response has no MCP result") from error
        structured = result.get("structuredContent") if isinstance(result, dict) else None
        text_payload = None
        try:
            content_text = result["content"][0]["text"]
            text_payload = json.loads(content_text)
        except (KeyError, IndexError, TypeError, json.JSONDecodeError):
            text_payload = None
        if isinstance(structured, dict):
            if isinstance(text_payload, dict) and text_payload != structured:
                raise SourceError(f"{name} text and structuredContent results diverged")
            return structured
        if isinstance(text_payload, dict):
            return text_payload
        raise SourceError(f"{name} response has no JSON object payload")

    def close(self) -> None:
        if self.process.stdin is not None and not self.process.stdin.closed:
            try:
                self.process.stdin.close()
            except OSError:
                pass
        try:
            return_code = self.process.wait(timeout=min(self.timeout_seconds, 10.0))
        except subprocess.TimeoutExpired:
            self._terminate()
            return_code = self.process.returncode
        self.stdout_reader.join(timeout=1)
        self.stderr_reader.join(timeout=1)
        for stream in (self.process.stdout, self.process.stderr):
            if stream is not None and not stream.closed:
                stream.close()
        if return_code not in (0, None):
            raise SourceError(
                f"packaged Unica MCP exited with {return_code}: {self._diagnostic_text()}"
            )


def _atomic_write_report(path: Path, report: dict) -> None:
    payload = (
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")
    temporary_path = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb",
            dir=path.parent,
            prefix=f".{path.name}.",
            delete=False,
        ) as stream:
            temporary_path = Path(stream.name)
            os.chmod(temporary_path, 0o600)
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_path, path)
    except OSError as error:
        if temporary_path is not None:
            try:
                temporary_path.unlink(missing_ok=True)
            except OSError:
                pass
        raise SourceError(f"cannot atomically write issue #76 report: {error}") from error


def _source_error_report(error: Exception, redactions) -> dict:
    sanitized_message = _sanitize_text(str(error), redactions)
    return _sanitize_value(
        {
            "schemaVersion": 1,
            "scenario": SCENARIO,
            "status": "source-error",
            "exitCode": 2,
            "steps": [],
            "partialGuard": {"blocked": False, "sourceUnchanged": False},
            "roundTrip": {
                "metadata": {"survived": False},
                "module": {"survived": False},
            },
            "configDumpInfo": {"informationalOnly": True},
            "summary": {"passed": False, "stepCount": 0},
            "sourceError": {"message": sanitized_message},
        },
        redactions,
        redact_tokens=False,
    )


def _packaged_manifest_provenance(
    plugin_root: Path,
    *,
    unica_binary_sha256: str,
) -> dict:
    if _SHA256_RE.fullmatch(unica_binary_sha256) is None:
        raise SourceError("executed Unica binary has an invalid sha256")
    path = _require_regular_file(
        plugin_root / "third-party/manifest.json",
        "packaged plugin tool manifest",
    )
    try:
        manifest_bytes = path.read_bytes()
        payload = json.loads(manifest_bytes)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SourceError(f"cannot read packaged plugin tool manifest: {error}") from error
    if not isinstance(payload, dict):
        raise SourceError("packaged plugin tool manifest must be a JSON object")
    schema_version = payload.get("schemaVersion")
    target_triple = payload.get("targetTriple")
    tools = payload.get("tools")
    if isinstance(schema_version, bool) or not isinstance(schema_version, int):
        raise SourceError("packaged plugin tool manifest has invalid schemaVersion")
    if not isinstance(target_triple, str) or not target_triple:
        raise SourceError("packaged plugin tool manifest has invalid targetTriple")
    if not isinstance(tools, list):
        raise SourceError("packaged plugin tool manifest has invalid tools array")

    selected = {}
    for item in tools:
        if not isinstance(item, dict) or item.get("name") not in _MANIFEST_TOOL_NAMES:
            continue
        name = item["name"]
        if name in selected:
            raise SourceError(f"packaged plugin tool manifest duplicates {name}")
        projection = {}
        for field in _MANIFEST_TOOL_FIELDS:
            value = item.get(field)
            if not isinstance(value, str) or not value:
                raise SourceError(
                    f"packaged plugin tool manifest {name} has invalid {field}"
                )
            projection[field] = value
        if _SHA256_RE.fullmatch(projection["sha256"]) is None:
            raise SourceError(
                f"packaged plugin tool manifest {name} has invalid sha256"
            )
        selected[name] = projection
    missing = sorted(set(_MANIFEST_TOOL_NAMES) - set(selected))
    if missing:
        raise SourceError(
            "packaged plugin tool manifest is missing required tools: "
            + ", ".join(missing)
        )
    if selected["unica"]["sha256"] != unica_binary_sha256:
        raise SourceError(
            "packaged plugin manifest Unica sha256 does not match the executed binary"
        )
    return {
        "sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        "schemaVersion": schema_version,
        "targetTriple": target_triple,
        "tools": selected,
    }


def _record_source_error(report: dict, message: str) -> None:
    report["status"] = "source-error"
    report["exitCode"] = 2
    summary = report.setdefault("summary", {})
    summary["passed"] = False
    failures = summary.setdefault("failures", [])
    if message not in failures:
        failures.append(message)
    report["sourceError"] = {"message": message}


def execute_gate(
    *,
    binary: Path,
    binary_args: list[str],
    plugin_root: Path,
    database: Path,
    sources: Path,
    parent_configuration: Path,
    platform_path: Path,
    platform_version: str,
    report_path: Path,
    evidence_dir: Path | None,
    builder: str,
    db_user: str,
    timeout_seconds: float,
    execute: bool,
    allow_empty_password: bool,
    session_factory=McpSession,
) -> tuple[int, dict]:
    if execute is not True or allow_empty_password is not True:
        raise SourceError("both live mutation opt-ins must be explicit")
    if builder not in {"DESIGNER", "IBCMD"}:
        raise SourceError("builder must be DESIGNER or IBCMD")
    if PLATFORM_VERSION_RE.fullmatch(platform_version) is None:
        raise SourceError("platform version must be exact 8.3.27.x")
    if (
        not math.isfinite(timeout_seconds)
        or timeout_seconds <= 0
        or timeout_seconds > MAX_TIMEOUT_SECONDS
    ):
        raise SourceError("timeout must be finite, positive, and no more than 24 hours")

    initial_redactions = [
        (database, "$DATABASE_INPUT"),
        (sources, "$SOURCE_INPUT"),
        (parent_configuration, "$PARENT_CONFIGURATION_INPUT"),
        (platform_path, "$PLATFORM"),
        (plugin_root, "$PLUGIN_ROOT"),
        (binary, "$UNICA_BINARY"),
        (db_user, "$DB_USER"),
    ]
    database_root = _resolved_absolute(database, "database input", directory=True)
    sources_root = _resolved_absolute(sources, "source input", directory=True)
    parent_configuration_path = _resolved_absolute(
        parent_configuration,
        "parent configuration input",
        directory=False,
    )
    binary_path = _resolved_absolute(binary, "Unica binary", directory=False)
    plugin = _resolved_absolute(plugin_root, "plugin root", directory=True)
    platform = _resolved_absolute(platform_path, "platform path", directory=True)
    if not os.access(binary_path, os.X_OK):
        raise SourceError("Unica binary is not executable")
    if _paths_overlap(database_root, sources_root):
        raise SourceError("database and source inputs must not overlap")
    if _is_relative_to(parent_configuration_path, database_root) or _is_relative_to(
        parent_configuration_path,
        sources_root,
    ):
        raise SourceError(
            "parent configuration input must be outside the database and source trees"
        )
    _require_regular_file(database_root / "1Cv8.1CD", "file infobase payload")
    for relative, label in (
        (Path("Configuration.xml"), "configuration descriptor"),
        (CATALOG_RELATIVE_PATH.relative_to("src"), "catalog descriptor"),
        (
            COMMON_MODULE_DESCRIPTOR_RELATIVE_PATH.relative_to("src"),
            "common module descriptor",
        ),
        (MODULE_RELATIVE_PATH.relative_to("src"), "common module source"),
    ):
        _require_regular_file(sources_root / relative, label)

    protected_mutation_paths = (
        (database_root, "the database input"),
        (sources_root, "the source input"),
        (parent_configuration_path, "the parent configuration input"),
        (binary_path, "the Unica executable"),
        (plugin, "the plugin root"),
        (platform, "the platform root"),
        (Path(__file__).resolve().parents[2], "the repository"),
    )
    report_target = _validate_report_path(
        report_path,
        protected_paths=protected_mutation_paths,
    )
    temporary = None
    evidence = None
    redactions = list(initial_redactions)
    report = None
    exit_code = 2
    integrity_probe = None
    private_binary_started = False

    try:
        binary_state_before = _regular_file_stat_signature(binary_path)
        binary_sha256_before = _hash_file(binary_path)
        plugin_manifest_path = plugin / "third-party/manifest.json"
        plugin_manifest = _packaged_manifest_provenance(
            plugin,
            unica_binary_sha256=binary_sha256_before,
        )
        plugin_manifest_state_before = _regular_file_stat_signature(
            plugin_manifest_path
        )
        if evidence_dir is None:
            temporary_parent = _safe_automatic_evidence_parent(
                protected_paths=protected_mutation_paths,
            )
            try:
                temporary = tempfile.TemporaryDirectory(
                    prefix="unica-issue-76-",
                    dir=temporary_parent,
                )
            except OSError as error:
                raise SourceError(f"cannot create private evidence directory: {error}") from error
            evidence = _validate_evidence_directory(
                Path(temporary.name),
                database=database_root,
                sources=sources_root,
                report_path=report_target,
                protected_paths=protected_mutation_paths,
            )
        else:
            evidence = _validate_evidence_directory(
                evidence_dir,
                database=database_root,
                sources=sources_root,
                report_path=report_target,
                protected_paths=protected_mutation_paths,
            )
        redactions.extend(
            [
                (evidence, "$EVIDENCE"),
                (evidence / "workspace", "$EVIDENCE/workspace"),
            ]
        )

        private_binary = evidence / "unica-executable"
        private_binary_sha256, private_binary_size = _copy_regular_file(
            binary_path,
            private_binary,
        )
        if private_binary_sha256 != binary_sha256_before:
            raise SourceError("Unica binary changed while making the private execution copy")
        private_binary_receipt = {
            "sha256": private_binary_sha256,
            "bytes": private_binary_size,
            "path": "$EVIDENCE/unica-executable",
        }

        database_state_before = _stat_tree_digest(database_root)
        source_state_before = _stat_tree_digest(sources_root)
        parent_state_before = _regular_file_stat_signature(parent_configuration_path)
        workspace = evidence / "workspace"
        workspace.mkdir(mode=0o700)
        database_copy = workspace / "ib"
        source_copy = workspace / "src"
        database_receipt = _copy_regular_tree(database_root, database_copy)
        source_receipt = _copy_regular_tree(sources_root, source_copy)
        parent_receipt = _install_parent_configuration(
            parent_configuration_path,
            source_copy,
        )
        (workspace / "work").mkdir(mode=0o700)
        cache = evidence / "cache"
        cache.mkdir(mode=0o700)
        runtime_platform = platform
        before_full_dump = None
        runtime_isolation = {
            "builder": builder,
            "privateIbcmdData": None,
            "buildPlatformPath": str(platform),
            "buildDataPath": None,
            "fullDumpPlatformPath": str(platform),
        }
        if builder == "IBCMD":
            runtime_platform, runtime_isolation = _create_private_ibcmd_platform(
                evidence,
                trusted_platform=platform,
                platform_version=platform_version,
            )

            def switch_to_trusted_full_dump_platform() -> None:
                _replace_project_platform_path(
                    workspace,
                    previous_path=runtime_platform,
                    next_path=platform,
                )

            before_full_dump = switch_to_trusted_full_dump_platform
        _write_project_configuration(
            workspace,
            database_copy=database_copy,
            platform_path=runtime_platform,
            platform_version=platform_version,
            db_user=db_user,
            builder=builder,
            timeout_seconds=timeout_seconds,
        )

        def inspect_input_integrity() -> dict:
            database_state_after = _stat_tree_digest(database_root)
            source_state_after = _stat_tree_digest(sources_root)
            parent_state_after = _regular_file_stat_signature(
                parent_configuration_path
            )
            parent_hash_after = _hash_file(parent_configuration_path)
            binary_state_after = _regular_file_stat_signature(binary_path)
            binary_sha256_after = _hash_file(binary_path)
            plugin_manifest_state_after = _regular_file_stat_signature(
                plugin_manifest_path
            )
            plugin_manifest_sha256_after = _hash_file(plugin_manifest_path)
            parent_unchanged = (
                parent_state_before == parent_state_after
                and parent_receipt["sha256"] == parent_hash_after
            )
            binary_unchanged = (
                binary_state_before == binary_state_after
                and binary_sha256_before == binary_sha256_after
            )
            plugin_manifest_unchanged = (
                plugin_manifest_state_before == plugin_manifest_state_after
                and plugin_manifest["sha256"] == plugin_manifest_sha256_after
            )
            database_unchanged = database_state_before == database_state_after
            sources_unchanged = source_state_before == source_state_after
            unchanged = (
                database_unchanged
                and sources_unchanged
                and parent_unchanged
                and binary_unchanged
                and plugin_manifest_unchanged
            )
            return {
                "unchanged": unchanged,
                "binaryUnchanged": binary_unchanged,
                "pluginManifestUnchanged": plugin_manifest_unchanged,
                "inputs": {
                    "database": {
                        "path": "$DATABASE_INPUT",
                        "copy": database_receipt,
                        "statUnchanged": database_unchanged,
                    },
                    "sources": {
                        "path": "$SOURCE_INPUT",
                        "copy": source_receipt,
                        "statUnchanged": sources_unchanged,
                    },
                    "parentConfiguration": {
                        "path": "$PARENT_CONFIGURATION_INPUT",
                        "copy": parent_receipt,
                        "statUnchanged": parent_state_before == parent_state_after,
                        "hashUnchanged": parent_receipt["sha256"]
                        == parent_hash_after,
                    },
                    "privateCopiesOnly": unchanged,
                },
            }

        integrity_probe = inspect_input_integrity

        environment = os.environ.copy()
        environment["UNICA_PLUGIN_ROOT"] = str(plugin)
        environment["UNICA_CACHE_DIR"] = str(cache)
        command = [str(private_binary), *binary_args]
        session = session_factory(
            command,
            environment,
            timeout_seconds,
            cwd=workspace,
        )
        private_binary_started = True
        close_error = None
        try:
            session.start(REQUIRED_TOOLS)
            exit_code, report = run_roundtrip_flow(
                session,
                workspace=workspace,
                redactions=redactions,
                before_full_dump=before_full_dump,
            )
        finally:
            try:
                session.close()
            except SourceError as error:
                close_error = error
            except Exception as error:
                close_error = SourceError(f"MCP session close failed: {error}")
        if close_error is not None:
            raise close_error

        integrity = integrity_probe()
        report["provenance"] = {
            "unicaBinarySha256": binary_sha256_before,
            "unicaBinaryCopy": private_binary_receipt,
            "executedPrivateBinaryCopy": private_binary_started,
            "unicaBinaryUnchanged": integrity["binaryUnchanged"],
            "pluginManifest": plugin_manifest,
            "pluginManifestUnchanged": integrity["pluginManifestUnchanged"],
            "platformVersion": platform_version,
            "platformPath": "$PLATFORM",
            "builder": builder,
        }
        report["runtimeIsolation"] = runtime_isolation
        report["inputs"] = integrity["inputs"]
        report["evidence"] = {
            "retained": evidence_dir is not None,
            "cleanupSucceeded": None if evidence_dir is not None else False,
            "workspace": "$EVIDENCE/workspace",
            "containsProprietaryParentConfiguration": True,
        }
        if not integrity["unchanged"]:
            exit_code = 2
            _record_source_error(
                report,
                "an input tree changed while the private live scenario ran",
            )
        report = _sanitize_value(report, redactions, redact_tokens=False)
    except Exception as error:
        exit_code = 2
        report = _source_error_report(error, redactions)
        if integrity_probe is not None:
            try:
                integrity = integrity_probe()
            except SourceError as integrity_error:
                _record_source_error(
                    report,
                    f"{report['sourceError']['message']}; input integrity check failed: {integrity_error}",
                )
            else:
                report["inputs"] = integrity["inputs"]
                report["provenance"] = {
                    "unicaBinarySha256": binary_sha256_before,
                    "unicaBinaryCopy": private_binary_receipt,
                    "executedPrivateBinaryCopy": private_binary_started,
                    "unicaBinaryUnchanged": integrity["binaryUnchanged"],
                    "pluginManifest": plugin_manifest,
                    "pluginManifestUnchanged": integrity[
                        "pluginManifestUnchanged"
                    ],
                    "platformVersion": platform_version,
                    "platformPath": "$PLATFORM",
                    "builder": builder,
                }
                report["runtimeIsolation"] = runtime_isolation
                if not integrity["unchanged"]:
                    _record_source_error(
                        report,
                        f"{report['sourceError']['message']}; an input tree changed while the private live scenario ran",
                    )
        if evidence is not None:
            private_parent = evidence / "workspace" / PARENT_CONFIGURATION_RELATIVE_PATH
            report["evidence"] = {
                "retained": evidence_dir is not None,
                "cleanupSucceeded": None if evidence_dir is not None else False,
                "workspace": "$EVIDENCE/workspace",
                "containsProprietaryParentConfiguration": private_parent.is_file(),
            }

    if temporary is not None:
        try:
            temporary.cleanup()
        except OSError as error:
            cleanup_message = f"temporary issue #76 evidence cleanup failed: {error}"
            exit_code = 2
            evidence_report = report.setdefault("evidence", {})
            evidence_report.update(
                {
                    "retained": True,
                    "cleanupSucceeded": False,
                    "containsProprietaryParentConfiguration": (
                        evidence is not None
                        and (
                            evidence
                            / "workspace"
                            / PARENT_CONFIGURATION_RELATIVE_PATH
                        ).is_file()
                    ),
                }
            )
            if evidence is not None:
                evidence_report["workspace"] = "$EVIDENCE/workspace"
            else:
                evidence_report.pop("workspace", None)
            prior_message = report.get("sourceError", {}).get("message")
            terminal_message = (
                f"{prior_message}; {cleanup_message}"
                if prior_message
                else cleanup_message
            )
            _record_source_error(report, terminal_message)
        else:
            evidence_report = report.setdefault("evidence", {})
            evidence_report["retained"] = False
            evidence_report["cleanupSucceeded"] = True
    report = _sanitize_value(report, redactions, redact_tokens=False)
    _atomic_write_report(report_target, report)
    return exit_code, report


def main(argv=None) -> int:
    arguments = _argument_parser().parse_args(argv)
    try:
        exit_code, report = execute_gate(
            binary=arguments.binary,
            binary_args=arguments.binary_arg,
            plugin_root=arguments.plugin_root,
            database=arguments.database,
            sources=arguments.sources,
            parent_configuration=arguments.parent_configuration,
            platform_path=arguments.platform_path,
            platform_version=arguments.platform_version,
            report_path=arguments.report,
            evidence_dir=arguments.evidence_dir,
            builder=arguments.builder,
            db_user=arguments.db_user,
            timeout_seconds=arguments.timeout_seconds,
            execute=arguments.execute,
            allow_empty_password=arguments.allow_empty_password,
        )
    except SourceError as error:
        print(f"source error: {error}", file=sys.stderr)
        return 2
    if exit_code != 0:
        failures = report.get("summary", {}).get("failures", [])
        detail = "; ".join(str(item) for item in failures) or report.get(
            "sourceError", {}
        ).get("message", "verification failed")
        print(f"issue #76 verification failed: {detail}", file=sys.stderr)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
