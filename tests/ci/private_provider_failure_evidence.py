"""Copy a bounded, closed provider failure record before an owned fixture is removed."""
from __future__ import annotations

from dataclasses import dataclass
import itertools
import json
import os
from pathlib import Path
import re
import stat
import sys
from typing import TextIO


RECORD_PREFIX = b"UNICA_PRIVATE_FAILURE_V1 "
EVIDENCE_PREFIX = "UNICA_PRIVATE_FAILURE_EVIDENCE_V1 "
READ_LIMIT = 16 * 1024
PHASE_CAUSES = {
    "write_request": frozenset({"cancelled", "deadline_elapsed", "writer_unavailable",
                                 "encode_request", "write_io", "flush_io"}),
    "wait_response": frozenset({"cancelled", "deadline_elapsed", "stdout_eof",
                                 "incomplete_frame", "invalid_json", "response_too_large",
                                 "read_io", "reader_disconnected"}),
    "decode_tool_result": frozenset({"cancelled", "remote_rpc_error",
                                      "remote_tool_error", "missing_result"}),
}
IO_KINDS = frozenset({"broken_pipe", "connection_reset", "connection_aborted",
                      "not_connected", "timed_out", "unexpected_eof", "interrupted",
                      "would_block", "permission_denied", "other"})
IO_CAUSES = frozenset({"write_io", "flush_io", "read_io"})
KEYS = frozenset({"version", "provider", "phase", "cause", "io_kind",
                  "service_pid", "helper_pid", "request_id"})
STATUSES = frozenset({"captured", "not_present", "unlocated", "ambiguous",
                      "truncated", "invalid_record", "io_unavailable"})


def _json(value) -> str:
    return json.dumps(value, ensure_ascii=True, separators=(",", ":"), allow_nan=False)


def _record_bound() -> int:
    return max(len(RECORD_PREFIX) + len(_json({
        "version": 1, "provider": "bsl_analyzer", "phase": phase, "cause": cause,
        "io_kind": kind, "service_pid": 2**32 - 1, "helper_pid": 2**32 - 1,
        "request_id": 2**63 - 1,
    })) + 1 for phase, causes in PHASE_CAUSES.items() for cause in causes
        for kind in (IO_KINDS if cause in IO_CAUSES else (None,)))


MAX_RECORD_BYTES = _record_bound()


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result


def _reject_constant(_):
    raise ValueError("nonfinite")


def _load(data: bytes):
    return json.loads(data, object_pairs_hook=_unique_object,
                      parse_constant=_reject_constant)


def validate_record(line: bytes) -> dict:
    """Accept complete formatter output; arbitrary log text never becomes evidence."""
    if (len(line) > MAX_RECORD_BYTES or not line.startswith(RECORD_PREFIX)
            or not line.endswith(b"\n")):
        raise ValueError("not a complete bounded record")
    record = _load(line[len(RECORD_PREFIX):-1])
    if not isinstance(record, dict) or record.keys() != KEYS:
        raise ValueError("wrong fields")
    if type(record["version"]) is not int or record["version"] != 1:
        raise ValueError("wrong version")
    if record["provider"] != "bsl_analyzer":
        raise ValueError("wrong provider")
    phase, cause, kind = record["phase"], record["cause"], record["io_kind"]
    if (not isinstance(phase, str) or not isinstance(cause, str)
            or cause not in PHASE_CAUSES.get(phase, ())):
        raise ValueError("wrong failure category")
    if cause in IO_CAUSES:
        if not isinstance(kind, str) or kind not in IO_KINDS:
            raise ValueError("wrong I/O category")
    elif kind is not None:
        raise ValueError("unexpected I/O category")
    for key, low, high in (("service_pid", 1, 2**32 - 1),
                           ("helper_pid", 1, 2**32 - 1), ("request_id", 2, 2**63 - 1)):
        if type(record[key]) is not int or not low <= record[key] <= high:
            raise ValueError("wrong numeric identity")
    return record


@dataclass(frozen=True)
class FailureCapture:
    status: str
    record: dict | None = None


@dataclass(frozen=True)
class FailureBaseline:
    status: str
    path: Path | None = None
    identity: tuple[int, int] | None = None
    offset: int = 0
    service_pid: int | None = None


class _Unavailable(Exception):
    def __init__(self, status: str):
        self.status = status


def _open_regular(path: Path):
    if stat.S_ISLNK(path.lstat().st_mode):
        raise _Unavailable("unlocated")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
    fd = os.open(path, flags)
    stream = os.fdopen(fd, "rb")
    if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
        stream.close()
        raise _Unavailable("unlocated")
    return stream


def _service(workspace: Path, cache_is_default: bool) -> tuple[Path, int]:
    if not cache_is_default:
        raise _Unavailable("unlocated")
    root = workspace.resolve(strict=True)
    if not ((root / "v8project.yaml").is_file() or (root / ".git").is_file()):
        raise _Unavailable("unlocated")
    directory = root
    for component in (".build", "unica", "services"):
        directory /= component
        mode = directory.lstat().st_mode
        if not stat.S_ISDIR(mode) or stat.S_ISLNK(mode):
            raise _Unavailable("unlocated")
    with os.scandir(directory) as entries:
        candidates = list(itertools.islice(entries, 2))
    if not candidates:
        raise _Unavailable("not_present")
    if len(candidates) != 1:
        raise _Unavailable("ambiguous")
    candidate = candidates[0]
    if (not re.fullmatch(r"svc-[0-9a-f]{16}", candidate.name)
            or not candidate.is_dir(follow_symlinks=False)):
        raise _Unavailable("unlocated")
    service = Path(candidate.path)
    with _open_regular(service / "service.json") as stream:
        data = stream.read(READ_LIMIT + 1)
    if len(data) > READ_LIMIT:
        raise _Unavailable("truncated")
    owner = _load(data)
    if (not isinstance(owner, dict) or type(owner.get("pid")) is not int
            or not 1 <= owner["pid"] <= 2**32 - 1
            or not isinstance(owner.get("workspace_root"), str)
            or os.path.normcase(os.path.normpath(owner["workspace_root"]))
            != os.path.normcase(str(root))):
        raise _Unavailable("unlocated")
    return service / "service.stderr.log", owner["pid"]


def before_call(workspace: Path, *, cache_is_default: bool) -> FailureBaseline:
    try:
        path, pid = _service(workspace, cache_is_default)
        with _open_regular(path) as stream:
            info = os.fstat(stream.fileno())
        return FailureBaseline("located", path, (info.st_dev, info.st_ino), info.st_size, pid)
    except FileNotFoundError:
        return FailureBaseline("not_present")
    except _Unavailable as error:
        return FailureBaseline(error.status)
    except (OSError, ValueError, UnicodeError, RecursionError):
        return FailureBaseline("io_unavailable")


def collect_failure(workspace: Path, baseline: FailureBaseline, *,
                    cache_is_default: bool) -> FailureCapture:
    try:
        if baseline.status not in {"located", "not_present"}:
            return FailureCapture(baseline.status)
        path, pid = _service(workspace, cache_is_default)
        with _open_regular(path) as stream:
            info = os.fstat(stream.fileno())
            if baseline.path is not None and (
                    baseline.path != path or baseline.service_pid != pid
                    or baseline.identity != (info.st_dev, info.st_ino)):
                return FailureCapture("ambiguous")
            if info.st_size < baseline.offset or info.st_size - baseline.offset > READ_LIMIT:
                return FailureCapture("truncated")
            stream.seek(baseline.offset)
            data = stream.read(READ_LIMIT + 1)
        if len(data) > READ_LIMIT:
            return FailureCapture("truncated")
        records = []
        for line in data.splitlines(keepends=True):
            if line.startswith(RECORD_PREFIX):
                record = validate_record(line)
                if record["service_pid"] != pid:
                    return FailureCapture("invalid_record")
                records.append(record)
                if len(records) > 1:
                    return FailureCapture("ambiguous")
        return (FailureCapture("captured", records[0]) if records
                else FailureCapture("not_present"))
    except FileNotFoundError:
        return FailureCapture("not_present")
    except _Unavailable as error:
        return FailureCapture(error.status)
    except (ValueError, UnicodeError, RecursionError):
        return FailureCapture("invalid_record")
    except OSError:
        return FailureCapture("io_unavailable")


def emit_failure_evidence(workspace: Path, baseline: FailureBaseline, *,
                          cache_is_default: bool, case_ordinal: int,
                          step_ordinal: int, writer: TextIO | None = None) -> None:
    capture = collect_failure(workspace, baseline, cache_is_default=cache_is_default)
    if capture.status not in STATUSES:
        capture = FailureCapture("invalid_record")
    evidence = {"case_ordinal": case_ordinal, "step_ordinal": step_ordinal,
                "capture_status": capture.status}
    if capture.record is not None:
        evidence["record"] = capture.record
    try:
        print(EVIDENCE_PREFIX + _json(evidence), file=writer if writer is not None else sys.stderr,
              flush=True)
    except (OSError, ValueError):
        pass
