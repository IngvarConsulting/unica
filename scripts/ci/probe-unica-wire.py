#!/usr/bin/env python3
"""Capture the deterministic initialize and tools/list wire surface of Unica."""

from __future__ import annotations

import argparse
import json
import os
import queue
import signal
import subprocess
import threading
import time
from pathlib import Path


_CLEANUP_GRACE_SECONDS = 0.5
_WINDOWS_CREATE_SUSPENDED = 0x00000004
EVIDENCE_TARGETS = ("darwin-arm64", "linux-x64", "win-x64")


class ProcessIdentity:
    """A PID plus the immutable evidence needed to reject PID reuse."""

    __slots__ = ("pid", "parent_pid", "session_id", "start_identity")

    def __init__(
        self,
        pid: int,
        parent_pid: int | None,
        session_id: int | None,
        start_identity: str,
    ) -> None:
        self.pid = pid
        self.parent_pid = parent_pid
        self.session_id = session_id
        self.start_identity = start_identity

    def __hash__(self) -> int:
        return hash(
            (self.pid, self.parent_pid, self.session_id, self.start_identity)
        )

    def __eq__(self, other: object) -> bool:
        return (
            isinstance(other, ProcessIdentity)
            and self.pid == other.pid
            and self.parent_pid == other.parent_pid
            and self.session_id == other.session_id
            and self.start_identity == other.start_identity
        )

    def identifies(self, other: ProcessIdentity | None) -> bool:
        return (
            other is not None
            and self.pid == other.pid
            and self.session_id == other.session_id
            and self.start_identity == other.start_identity
        )


class ProcessCleanupResult:
    """Bounded cleanup evidence, including failures that prevent a clean claim."""

    __slots__ = ("survivors", "incomplete", "active_processes")

    def __init__(
        self,
        survivors: set[ProcessIdentity],
        incomplete: tuple[str, ...] = (),
        active_processes: int | None = None,
    ) -> None:
        self.survivors = survivors
        self.incomplete = incomplete
        self.active_processes = active_processes

    @property
    def complete(self) -> bool:
        return not self.survivors and not self.incomplete


class ProcessOwnership:
    """Tracks one spawned session and identities registered before escape.

    Windows ownership is a retained Job Object assigned while the leader is
    suspended; numeric identities there are diagnostics, never kill authority.
    An unregistered process that calls ``setsid()`` has intentionally left the
    only ownership boundary available to an unprivileged POSIX parent. It is
    never rediscovered from a late bare PID. Callers that intentionally create
    such a daemon must register its exact identity before the session escape.
    """

    def __init__(
        self,
        process: subprocess.Popen[str],
        public_identity: ProcessIdentity | None,
        session_id: int | None,
        windows_job: object | None = None,
    ) -> None:
        self.process = process
        self.public_identity = public_identity
        self.session_id = session_id
        self.windows_job = windows_job

    @classmethod
    def capture(
        cls,
        process: subprocess.Popen[str],
        cleanup_deadline: float | None = None,
    ) -> ProcessOwnership:
        pid = getattr(process, "pid", 0)
        windows_job = (
            _attach_windows_process_job(process, deadline=cleanup_deadline)
            if os.name == "nt"
            else None
        )
        identity = _current_process_identity(pid)
        session_id = pid if os.name == "posix" and pid > 0 else None
        if identity is not None and identity.session_id is not None:
            session_id = identity.session_id
        return cls(process, identity, session_id, windows_job)

    def snapshot(
        self, additional_identities: set[ProcessIdentity] | None = None
    ) -> set[ProcessIdentity]:
        additional_identities = set(additional_identities or ())
        if os.name == "posix":
            processes = _posix_process_snapshot()
            owned = {
                identity
                for identity in processes.values()
                if self.session_id is not None
                and identity.session_id == self.session_id
            }
            roots = {
                identity
                for identity in additional_identities
                if identity.identifies(processes.get(identity.pid))
            }
            owned.update(roots)
            while True:
                owned_pids = {identity.pid for identity in owned}
                descendants = {
                    identity
                    for identity in processes.values()
                    if identity.parent_pid in owned_pids
                }
                expanded = owned | descendants
                if expanded == owned:
                    return owned
                owned = expanded
        identities = {
            identity
            for identity in additional_identities
            if identity.identifies(_current_process_identity(identity.pid))
        }
        if self.public_identity is not None:
            identities.add(self.public_identity)
        return identities

    def capture_identities(self, pids: set[int]) -> set[ProcessIdentity]:
        return {
            identity
            for pid in pids
            if (identity := _current_process_identity(pid)) is not None
        }

    def signal(
        self, identities: set[ProcessIdentity], signal_number: int
    ) -> None:
        for identity in sorted(identities, key=lambda item: item.pid, reverse=True):
            if not identity.identifies(_current_process_identity(identity.pid)):
                continue
            try:
                os.kill(identity.pid, signal_number)
            except OSError:
                pass

    def survivors(
        self, identities: set[ProcessIdentity]
    ) -> set[ProcessIdentity]:
        return {
            identity
            for identity in identities
            if identity.identifies(_current_process_identity(identity.pid))
            and _process_is_running(identity.pid)
        }

    def wait(
        self, identities: set[ProcessIdentity], timeout_seconds: float
    ) -> set[ProcessIdentity]:
        deadline = time.monotonic() + max(0.0, timeout_seconds)
        survivors = self.survivors(identities)
        while survivors and time.monotonic() < deadline:
            time.sleep(min(0.01, max(0.0, deadline - time.monotonic())))
            survivors = self.survivors(survivors)
        return survivors

    def quiesce(
        self,
        identities: set[ProcessIdentity],
        timeout_seconds: float = _CLEANUP_GRACE_SECONDS,
    ) -> ProcessCleanupResult:
        cleanup_timeout = max(0.0, timeout_seconds)
        cleanup_deadline = time.monotonic() + cleanup_timeout
        if os.name == "nt":
            incomplete: list[str] = []
            active_processes: int | None = None
            job = getattr(self, "windows_job", None)
            if job is None:
                incomplete.append("Windows Job Object ownership is unavailable")
            else:
                termination_error = job.terminate()
                if termination_error is not None:
                    incomplete.append(termination_error)
                incomplete.extend(job.wait_for_leader_and_release(cleanup_deadline))
                while True:
                    active_processes, query_error = job.active_process_count()
                    if query_error is not None:
                        incomplete.append(query_error)
                        break
                    if active_processes == 0:
                        break
                    remaining = cleanup_deadline - time.monotonic()
                    if remaining <= 0:
                        incomplete.append(
                            "Windows Job Object still owns "
                            f"{active_processes} active process(es)"
                        )
                        break
                    time.sleep(min(0.01, remaining))
                close_error = job.close()
                if close_error is not None:
                    incomplete.append(close_error)
            survivors = self.wait(
                identities, max(0.0, cleanup_deadline - time.monotonic())
            )
            return ProcessCleanupResult(
                survivors,
                tuple(dict.fromkeys(incomplete)),
                active_processes,
            )

        survivors = self.survivors(identities)
        if survivors:
            self.signal(survivors, signal.SIGTERM)
            survivors = self.wait(
                survivors,
                min(0.1, max(0.0, cleanup_deadline - time.monotonic())),
            )
            if survivors:
                self.signal(survivors, signal.SIGKILL)
        try:
            self.process.wait(
                timeout=max(0.0, cleanup_deadline - time.monotonic())
            )
        except (OSError, subprocess.TimeoutExpired):
            pass
        return ProcessCleanupResult(
            self.wait(
                survivors, max(0.0, cleanup_deadline - time.monotonic())
            )
        )

    def terminate(
        self,
        additional_identities: set[ProcessIdentity] | None = None,
        timeout_seconds: float = _CLEANUP_GRACE_SECONDS,
    ) -> ProcessCleanupResult:
        return self.quiesce(
            self.snapshot(additional_identities), timeout_seconds
        )


def _response_kind(response: dict) -> str:
    if "error" in response:
        return "error"
    if "result" in response:
        return "result"
    return "other"


class JsonRpcSession:
    """One stdio JSON-RPC process governed by one caller-owned deadline."""

    error_label = "Unica MCP wire probe"

    def __init__(
        self,
        command: list[str],
        environment: dict[str, str],
        *,
        cwd: Path,
        deadline: float,
        timeout_seconds: float | None = None,
    ) -> None:
        popen_options = {}
        if os.name == "posix":
            popen_options["start_new_session"] = True
        elif os.name == "nt":
            popen_options["creationflags"] = (
                getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
                | _WINDOWS_CREATE_SUSPENDED
            )
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            env=environment,
            cwd=cwd,
            **popen_options,
        )
        attach_cleanup_deadline = min(
            deadline, time.monotonic() + _CLEANUP_GRACE_SECONDS
        )
        try:
            self.process_ownership = ProcessOwnership.capture(
                self.process, attach_cleanup_deadline
            )
        except BaseException as error:
            cleanup_errors: list[str] = []
            if os.name != "nt" or getattr(self.process, "_handle", None) is not None:
                try:
                    self.process.terminate()
                except OSError as cleanup_error:
                    cleanup_errors.append(
                        f"fallback process termination failed: {cleanup_error}"
                    )
                try:
                    self.process.wait(
                        timeout=max(
                            0.0,
                            attach_cleanup_deadline - time.monotonic(),
                        )
                    )
                except (OSError, subprocess.TimeoutExpired) as cleanup_error:
                    cleanup_errors.append(
                        f"fallback process wait failed: {cleanup_error}"
                    )
            cleanup_errors.extend(self._close_streams())
            if cleanup_errors:
                raise RuntimeError(
                    f"{error}; constructor cleanup incomplete: "
                    + "; ".join(cleanup_errors)
                ) from error
            raise
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        assert self.process.stderr is not None
        self.deadline = deadline
        self.timeout_seconds = timeout_seconds
        self.lines: queue.Queue[str] = queue.Queue()
        self.diagnostics: list[str] = []
        self.response_kinds: list[dict[str, str]] = []
        self.reader = threading.Thread(target=self._read_stdout, daemon=True)
        self.error_reader = threading.Thread(target=self._read_stderr, daemon=True)
        started_readers: list[threading.Thread] = []
        try:
            self.reader.start()
            started_readers.append(self.reader)
            self.error_reader.start()
            started_readers.append(self.error_reader)
        except BaseException:
            self.process_ownership.terminate()
            for reader in started_readers:
                reader.join(timeout=_CLEANUP_GRACE_SECONDS)
            self._close_streams()
            raise

    def _read_stdout(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put("")

    def _read_stderr(self) -> None:
        assert self.process.stderr is not None
        for line in self.process.stderr:
            self.diagnostics.append(line)

    def _detail(self) -> str:
        return "".join(self.diagnostics).strip() or "no process output"

    def _remaining_timeout(self) -> float:
        deadline = getattr(self, "deadline", None)
        if deadline is None:
            timeout_seconds = getattr(self, "timeout_seconds", None)
            if timeout_seconds is not None:
                return timeout_seconds
            raise SystemExit(f"{self.error_label} has no aggregate deadline")
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise SystemExit(
                f"{self.error_label} exceeded its aggregate deadline: {self._detail()}"
            )
        timeout_seconds = getattr(self, "timeout_seconds", None)
        return min(remaining, timeout_seconds) if timeout_seconds is not None else remaining

    def _request_timeout_message(self) -> str:
        timeout_seconds = getattr(self, "timeout_seconds", None)
        if timeout_seconds is not None:
            return (
                f"{self.error_label} timed out after {timeout_seconds:g}s: "
                f"{self._detail()}"
            )
        return f"{self.error_label} exceeded its aggregate deadline: {self._detail()}"

    def request(self, message: dict) -> dict:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()
        request_deadline = getattr(self, "deadline", None) or float("inf")
        timeout_seconds = getattr(self, "timeout_seconds", None)
        if timeout_seconds is not None:
            request_deadline = min(
                request_deadline, time.monotonic() + timeout_seconds
            )
        while True:
            remaining = request_deadline - time.monotonic()
            if remaining <= 0:
                raise SystemExit(self._request_timeout_message())
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty as error:
                raise SystemExit(self._request_timeout_message()) from error
            if not line:
                raise SystemExit(
                    f"{self.error_label} exited before the expected response: {self._detail()}"
                )
            try:
                response = json.loads(line)
            except json.JSONDecodeError as error:
                raise SystemExit(
                    f"{self.error_label} emitted invalid JSON: {error}: {line}"
                ) from error
            if not isinstance(response, dict):
                raise SystemExit(f"{self.error_label} emitted a non-object response: {line}")
            if response.get("id") == message.get("id"):
                self.response_kinds.append(
                    {
                        "kind": _response_kind(response),
                        "method": str(message.get("method", "")),
                    }
                )
                return response

    def notify(self, message: dict) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()

    def close(self) -> None:
        if self.process.stdin is not None and not self.process.stdin.closed:
            self.process.stdin.close()
        try:
            remaining = self._remaining_timeout()
            result = self.process.wait(timeout=remaining)
        except subprocess.TimeoutExpired as error:
            self._terminate_owned()
            raise SystemExit(
                f"{self.error_label} exceeded its aggregate deadline: {self._detail()}"
            ) from error
        except BaseException:
            self._terminate_owned()
            raise
        remaining = max(0.0, self.deadline - time.monotonic())
        self.reader.join(timeout=remaining)
        self.error_reader.join(timeout=remaining)
        if self.reader.is_alive() or self.error_reader.is_alive():
            self._terminate_owned()
            raise SystemExit(
                f"{self.error_label} reader threads did not stop before the aggregate deadline: "
                f"{self._detail()}"
            )
        detail = self._detail()
        self._close_streams()
        if result != 0:
            raise SystemExit(f"{self.error_label} exited with {result}: {detail}")

    def terminate_tree(
        self, additional_identities: set[ProcessIdentity] | None = None
    ) -> None:
        self._terminate_owned(additional_identities)

    def _terminate_owned(
        self, additional_identities: set[ProcessIdentity] | None = None
    ) -> None:
        cleanup_deadline = time.monotonic() + _CLEANUP_GRACE_SECONDS
        self._process_ownership().terminate(
            additional_identities,
            max(0.0, cleanup_deadline - time.monotonic()),
        )
        for reader_name in ("reader", "error_reader"):
            reader = getattr(self, reader_name, None)
            if reader is not None:
                reader.join(
                    timeout=max(0.0, cleanup_deadline - time.monotonic())
                )
        self._close_streams()

    def _process_ownership(self) -> ProcessOwnership:
        ownership = getattr(self, "process_ownership", None)
        if ownership is None:
            ownership = ProcessOwnership.capture(self.process)
            self.process_ownership = ownership
        return ownership

    def _close_streams(self) -> tuple[str, ...]:
        errors: list[str] = []
        streams = (
            (getattr(self.process, "stdin", None), None),
            (getattr(self.process, "stdout", None), getattr(self, "reader", None)),
            (getattr(self.process, "stderr", None), getattr(self, "error_reader", None)),
        )
        for stream, reader in streams:
            if reader is not None and reader.is_alive():
                continue
            if stream is not None and not stream.closed:
                try:
                    stream.close()
                except (OSError, ValueError) as error:
                    errors.append(f"stdio close failed: {error}")
        return tuple(errors)


class WireProbe:
    def __init__(
        self,
        command: list[str],
        *,
        protocol_version: str,
        tasks_capability: str,
        profile: str | None = None,
        target: str | None = None,
        timeout_seconds: float,
        environment: dict[str, str] | None = None,
        cwd: Path | None = None,
    ) -> None:
        self.command = command
        self.protocol_version = protocol_version
        self.tasks_capability = tasks_capability
        if profile is not None and profile not in {"native", "compatibility"}:
            raise ValueError(f"unsupported wire evidence profile: {profile}")
        if (profile is None) != (target is None):
            raise ValueError("wire evidence profile and target must be provided together")
        if target is not None and target not in EVIDENCE_TARGETS:
            raise ValueError(f"unsupported wire evidence target: {target}")
        self.profile = profile
        self.target = target
        self.timeout_seconds = timeout_seconds
        self.environment = dict(environment or os.environ)
        self.cwd = (cwd or Path.cwd()).resolve()

    def run(self) -> dict:
        deadline = time.monotonic() + self.timeout_seconds
        session = JsonRpcSession(
            self.command,
            self.environment,
            cwd=self.cwd,
            deadline=deadline,
        )
        completed = False
        try:
            modern = self.protocol_version == "2026-07-28"
            client_capabilities = self._client_capabilities()
            server_info: dict | None = None
            server_protocol_version = self.protocol_version
            request_id = 1
            if not modern:
                initialized = session.request(
                    {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "method": "initialize",
                        "params": {
                            "protocolVersion": self.protocol_version,
                            "capabilities": client_capabilities,
                            "clientInfo": {
                                "name": "unica-wire-probe",
                                "version": "1",
                            },
                        },
                    }
                )
                initialize_result = _require_result(initialized, "initialize")
                server_info_value = initialize_result.get("serverInfo")
                if not isinstance(server_info_value, dict):
                    raise SystemExit(
                        "Unica MCP wire probe initialize has no serverInfo object"
                    )
                server_info = server_info_value
                negotiated = initialize_result.get("protocolVersion")
                if not isinstance(negotiated, str):
                    raise SystemExit(
                        "Unica MCP wire probe initialize has no protocolVersion string"
                    )
                server_protocol_version = negotiated
                session.notify(
                    {
                        "jsonrpc": "2.0",
                        "method": "notifications/initialized",
                        "params": {},
                    }
                )
                request_id += 1

            cursor: str | None = None
            seen_cursors: set[str] = set()
            tool_names: list[str] = []
            seen_names: set[str] = set()
            while True:
                params = (
                    {"_meta": self._request_meta(client_capabilities)}
                    if modern
                    else {}
                )
                if cursor is not None:
                    params["cursor"] = cursor
                listed = session.request(
                    {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "method": "tools/list",
                        "params": params,
                    }
                )
                result = _require_result(listed, "tools/list")
                if modern and server_info is None:
                    result_meta = result.get("_meta")
                    modern_server_info = (
                        result_meta.get("io.modelcontextprotocol/serverInfo")
                        if isinstance(result_meta, dict)
                        else None
                    )
                    if isinstance(modern_server_info, dict):
                        server_info = modern_server_info
                tools = result.get("tools")
                if not isinstance(tools, list):
                    raise SystemExit("Unica MCP wire probe tools/list has no tools array")
                for tool in tools:
                    name = tool.get("name") if isinstance(tool, dict) else None
                    if not isinstance(name, str) or not name:
                        raise SystemExit(
                            f"Unica MCP wire probe tools/list has malformed tool entry: {tool!r}"
                        )
                    if name in seen_names:
                        raise SystemExit(
                            f"Unica MCP wire probe found duplicate tool name: {name}"
                        )
                    seen_names.add(name)
                    tool_names.append(name)
                next_cursor = result.get("nextCursor")
                if next_cursor is None:
                    break
                if not isinstance(next_cursor, str) or not next_cursor:
                    raise SystemExit(
                        "Unica MCP wire probe tools/list returned a malformed nextCursor"
                    )
                if next_cursor in seen_cursors:
                    raise SystemExit(
                        f"Unica MCP wire probe tools/list repeated cursor: {next_cursor}"
                    )
                seen_cursors.add(next_cursor)
                cursor = next_cursor
                request_id += 1

            output = {
                "schemaVersion": 1 if self.profile is not None else None,
                "protocolVersion": self.protocol_version,
                "responseKinds": session.response_kinds,
                "serverInfo": server_info,
                "serverProtocolVersion": server_protocol_version,
                "tasksCapability": self.tasks_capability,
                "toolCount": len(tool_names),
                "toolNames": sorted(tool_names),
            }
            if self.profile is None:
                output.pop("schemaVersion")
            else:
                output["profile"] = self.profile
                output["target"] = self.target
            completed = True
            return output
        finally:
            if completed:
                session.close()
            else:
                session.terminate_tree()

    def _client_capabilities(self) -> dict:
        if self.tasks_capability == "off":
            return {}
        return {"extensions": {"io.modelcontextprotocol/tasks": {}}}

    def _request_meta(self, client_capabilities: dict) -> dict:
        return {
            "io.modelcontextprotocol/protocolVersion": self.protocol_version,
            "io.modelcontextprotocol/clientInfo": {
                "name": "unica-wire-probe",
                "version": "1",
            },
            "io.modelcontextprotocol/clientCapabilities": client_capabilities,
        }


def _require_result(response: dict, method: str) -> dict:
    if "error" in response:
        raise SystemExit(f"Unica MCP wire probe {method} failed: {response['error']}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise SystemExit(f"Unica MCP wire probe {method} response is missing")
    return result


class _WindowsApi:
    """Small checked Win32 facade; tests inject the same operation surface."""

    def __init__(self) -> None:
        import ctypes
        from ctypes import wintypes

        class LargeInteger(ctypes.Structure):
            _fields_ = [("QuadPart", ctypes.c_longlong)]

        class IoCounters(ctypes.Structure):
            _fields_ = [
                ("ReadOperationCount", ctypes.c_ulonglong),
                ("WriteOperationCount", ctypes.c_ulonglong),
                ("OtherOperationCount", ctypes.c_ulonglong),
                ("ReadTransferCount", ctypes.c_ulonglong),
                ("WriteTransferCount", ctypes.c_ulonglong),
                ("OtherTransferCount", ctypes.c_ulonglong),
            ]

        class JobBasicLimitInformation(ctypes.Structure):
            _fields_ = [
                ("PerProcessUserTimeLimit", LargeInteger),
                ("PerJobUserTimeLimit", LargeInteger),
                ("LimitFlags", wintypes.DWORD),
                ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t),
                ("ActiveProcessLimit", wintypes.DWORD),
                ("Affinity", ctypes.c_size_t),
                ("PriorityClass", wintypes.DWORD),
                ("SchedulingClass", wintypes.DWORD),
            ]

        class JobExtendedLimitInformation(ctypes.Structure):
            _fields_ = [
                ("BasicLimitInformation", JobBasicLimitInformation),
                ("IoInfo", IoCounters),
                ("ProcessMemoryLimit", ctypes.c_size_t),
                ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t),
                ("PeakJobMemoryUsed", ctypes.c_size_t),
            ]

        class JobBasicAccountingInformation(ctypes.Structure):
            _fields_ = [
                ("TotalUserTime", ctypes.c_longlong),
                ("TotalKernelTime", ctypes.c_longlong),
                ("ThisPeriodTotalUserTime", ctypes.c_longlong),
                ("ThisPeriodTotalKernelTime", ctypes.c_longlong),
                ("TotalPageFaultCount", wintypes.DWORD),
                ("TotalProcesses", wintypes.DWORD),
                ("ActiveProcesses", wintypes.DWORD),
                ("TotalTerminatedProcesses", wintypes.DWORD),
            ]

        class ThreadEntry32(ctypes.Structure):
            _fields_ = [
                ("dwSize", wintypes.DWORD),
                ("cntUsage", wintypes.DWORD),
                ("th32ThreadID", wintypes.DWORD),
                ("th32OwnerProcessID", wintypes.DWORD),
                ("tpBasePri", wintypes.LONG),
                ("tpDeltaPri", wintypes.LONG),
                ("dwFlags", wintypes.DWORD),
            ]

        self.ctypes = ctypes
        self.wintypes = wintypes
        self.kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        self.JobExtendedLimitInformation = JobExtendedLimitInformation
        self.JobBasicAccountingInformation = JobBasicAccountingInformation
        self.ThreadEntry32 = ThreadEntry32

        signatures = (
            ("CreateJobObjectW", [ctypes.c_void_p, wintypes.LPCWSTR], wintypes.HANDLE),
            (
                "SetInformationJobObject",
                [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD],
                wintypes.BOOL,
            ),
            (
                "AssignProcessToJobObject",
                [wintypes.HANDLE, wintypes.HANDLE],
                wintypes.BOOL,
            ),
            ("TerminateJobObject", [wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
            ("TerminateProcess", [wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
            (
                "WaitForSingleObject",
                [wintypes.HANDLE, wintypes.DWORD],
                wintypes.DWORD,
            ),
            ("CloseHandle", [wintypes.HANDLE], wintypes.BOOL),
            (
                "CreateToolhelp32Snapshot",
                [wintypes.DWORD, wintypes.DWORD],
                wintypes.HANDLE,
            ),
            (
                "Thread32First",
                [wintypes.HANDLE, ctypes.POINTER(ThreadEntry32)],
                wintypes.BOOL,
            ),
            (
                "Thread32Next",
                [wintypes.HANDLE, ctypes.POINTER(ThreadEntry32)],
                wintypes.BOOL,
            ),
            (
                "OpenThread",
                [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD],
                wintypes.HANDLE,
            ),
            ("ResumeThread", [wintypes.HANDLE], wintypes.DWORD),
            (
                "QueryInformationJobObject",
                [
                    wintypes.HANDLE,
                    ctypes.c_int,
                    ctypes.c_void_p,
                    wintypes.DWORD,
                    ctypes.POINTER(wintypes.DWORD),
                ],
                wintypes.BOOL,
            ),
        )
        for name, argtypes, restype in signatures:
            function = getattr(self.kernel32, name)
            function.argtypes = argtypes
            function.restype = restype

    def _failure(self, operation: str) -> RuntimeError:
        return RuntimeError(
            f"{operation} failed with Windows error {self.ctypes.get_last_error()}"
        )

    def create_job(self) -> object:
        handle = self.kernel32.CreateJobObjectW(None, None)
        if not handle:
            raise self._failure("CreateJobObjectW")
        return handle

    def set_kill_on_job_close(self, job: object) -> None:
        limits = self.JobExtendedLimitInformation()
        limits.BasicLimitInformation.LimitFlags = 0x00002000
        if not self.kernel32.SetInformationJobObject(
            job,
            9,
            self.ctypes.byref(limits),
            self.ctypes.sizeof(limits),
        ):
            raise self._failure("SetInformationJobObject")

    def assign_process(self, job: object, process_handle: object) -> None:
        if not self.kernel32.AssignProcessToJobObject(job, process_handle):
            raise self._failure("AssignProcessToJobObject")

    def create_thread_snapshot(self) -> object:
        handle = self.kernel32.CreateToolhelp32Snapshot(0x00000004, 0)
        if handle == self.ctypes.c_void_p(-1).value:
            raise self._failure("CreateToolhelp32Snapshot")
        return handle

    def open_primary_thread(self, snapshot: object, pid: int) -> object:
        entry = self.ThreadEntry32()
        entry.dwSize = self.ctypes.sizeof(entry)
        if not self.kernel32.Thread32First(snapshot, self.ctypes.byref(entry)):
            raise self._failure("Thread32First")
        while True:
            if entry.th32OwnerProcessID == pid:
                handle = self.kernel32.OpenThread(
                    0x0002, False, entry.th32ThreadID
                )
                if not handle:
                    raise self._failure("OpenThread")
                return handle
            if not self.kernel32.Thread32Next(
                snapshot, self.ctypes.byref(entry)
            ):
                error = self.ctypes.get_last_error()
                if error == 18:
                    raise RuntimeError(
                        f"OpenThread found no primary thread for process {pid}"
                    )
                raise self._failure("Thread32Next")

    def resume_thread(self, thread: object) -> int:
        suspend_count = int(self.kernel32.ResumeThread(thread))
        if suspend_count == 0xFFFFFFFF:
            raise self._failure("ResumeThread")
        return suspend_count

    def terminate_process(self, process_handle: object) -> None:
        if not self.kernel32.TerminateProcess(process_handle, 1):
            raise self._failure("TerminateProcess")

    def terminate_job(self, job: object) -> None:
        if not self.kernel32.TerminateJobObject(job, 1):
            raise self._failure("TerminateJobObject")

    def wait_for_single_object(
        self, handle: object, timeout_seconds: float
    ) -> bool:
        milliseconds = min(
            0xFFFFFFFE, int(max(0.0, timeout_seconds) * 1000)
        )
        result = int(self.kernel32.WaitForSingleObject(handle, milliseconds))
        if result == 0:
            return True
        if result == 0x00000102:
            return False
        if result == 0xFFFFFFFF:
            raise self._failure("WaitForSingleObject")
        raise RuntimeError(
            f"WaitForSingleObject returned unexpected status {result}"
        )

    def close_popen_handle(self, process: subprocess.Popen[str]) -> None:
        handle = getattr(process, "_handle", None)
        if not handle:
            return
        try:
            close = getattr(handle, "Close", None)
            if close is None:
                self.close_handle(handle)
            else:
                close()
        except (OSError, RuntimeError) as error:
            raise RuntimeError(f"CloseHandle(process) failed: {error}") from error
        process._handle = None
        if getattr(process, "returncode", None) is None:
            process.returncode = 1

    def close_handle(self, handle: object) -> None:
        if not self.kernel32.CloseHandle(handle):
            raise self._failure("CloseHandle")

    def active_process_count(self, job: object) -> int:
        information = self.JobBasicAccountingInformation()
        if not self.kernel32.QueryInformationJobObject(
            job,
            1,
            self.ctypes.byref(information),
            self.ctypes.sizeof(information),
            None,
        ):
            raise self._failure("QueryInformationJobObject")
        return int(information.ActiveProcesses)


class _WindowsProcessJob:
    """A handle-bound Windows process tree created before the leader runs."""

    __slots__ = ("handle", "api", "process")

    def __init__(
        self,
        handle: object,
        api: object,
        process: subprocess.Popen[str],
    ) -> None:
        self.handle = handle
        self.api = api
        self.process = process

    def terminate(self) -> str | None:
        if not self.handle:
            return "Windows Job Object handle is closed"
        try:
            self.api.terminate_job(self.handle)
        except (OSError, RuntimeError) as error:
            return str(error)
        return None

    def wait_for_leader_and_release(self, deadline: float) -> tuple[str, ...]:
        errors: list[str] = []
        handle = getattr(self.process, "_handle", None)
        if handle:
            remaining = max(0.0, deadline - time.monotonic())
            try:
                if not self.api.wait_for_single_object(handle, remaining):
                    errors.append(
                        "WaitForSingleObject timed out before the spawned "
                        "Windows process exited"
                    )
            except (OSError, RuntimeError) as error:
                errors.append(str(error))
            try:
                self.api.close_popen_handle(self.process)
            except (OSError, RuntimeError) as error:
                errors.append(str(error))
        elif getattr(self.process, "returncode", None) is None:
            errors.append("spawned Windows process handle is unavailable")
        return tuple(errors)

    def active_process_count(self) -> tuple[int | None, str | None]:
        if not self.handle:
            return None, "Windows Job Object handle is closed"
        try:
            return self.api.active_process_count(self.handle), None
        except (OSError, RuntimeError) as error:
            return None, str(error)

    def close(self) -> str | None:
        if not self.handle:
            return None
        try:
            self.api.close_handle(self.handle)
        except (OSError, RuntimeError) as error:
            return str(error)
        self.handle = None
        return None

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass


def _rollback_windows_process_job_attach(
    process: subprocess.Popen[str],
    process_handle: object,
    job_handle: object | None,
    assigned_to_job: bool,
    resource_handles: list[tuple[str, object]],
    deadline: float,
    api: object,
) -> tuple[str, ...]:
    errors: list[str] = []

    try:
        api.terminate_process(process_handle)
    except (OSError, RuntimeError) as error:
        errors.append(str(error))
    if job_handle is not None and assigned_to_job:
        try:
            api.terminate_job(job_handle)
        except (OSError, RuntimeError) as error:
            errors.append(str(error))
    try:
        remaining = max(0.0, deadline - time.monotonic())
        if not api.wait_for_single_object(process_handle, remaining):
            errors.append("WaitForSingleObject timed out during Windows attach rollback")
    except (OSError, RuntimeError) as error:
        errors.append(str(error))
    try:
        api.close_popen_handle(process)
    except (OSError, RuntimeError) as error:
        errors.append(str(error))

    for label, handle in resource_handles:
        try:
            api.close_handle(handle)
        except (OSError, RuntimeError) as error:
            errors.append(f"CloseHandle({label}) failed: {error}")
    if job_handle is not None:
        try:
            api.close_handle(job_handle)
        except (OSError, RuntimeError) as error:
            errors.append(f"CloseHandle(job) failed: {error}")
    return tuple(errors)


def _attach_windows_process_job(
    process: subprocess.Popen[str],
    *,
    deadline: float | None = None,
    api: object | None = None,
) -> _WindowsProcessJob:
    """Assign a suspended process to a kill-on-close job, then resume it."""

    cleanup_deadline = (
        time.monotonic() + _CLEANUP_GRACE_SECONDS
        if deadline is None
        else deadline
    )
    windows_api = api if api is not None else _WindowsApi()
    process_handle = getattr(process, "_handle", None)
    if not process_handle:
        raise RuntimeError("spawned Windows process has no retained process handle")

    job_handle: object | None = None
    snapshot: object | None = None
    thread_handle: object | None = None
    assigned_to_job = False
    failure: BaseException | None = None
    try:
        job_handle = windows_api.create_job()
        windows_api.set_kill_on_job_close(job_handle)
        windows_api.assign_process(job_handle, process_handle)
        assigned_to_job = True
        snapshot = windows_api.create_thread_snapshot()
        thread_handle = windows_api.open_primary_thread(snapshot, process.pid)
        previous_suspend_count = windows_api.resume_thread(thread_handle)
        if previous_suspend_count != 1:
            raise RuntimeError(
                "unexpected primary thread suspend count: "
                f"{previous_suspend_count}"
            )
    except BaseException as error:
        failure = error

    resource_handles = [
        (label, handle)
        for label, handle in (
            ("thread", thread_handle),
            ("snapshot", snapshot),
        )
        if handle is not None
    ]
    if failure is None:
        close_errors: list[str] = []
        unclosed: list[tuple[str, object]] = []
        for label, handle in resource_handles:
            try:
                windows_api.close_handle(handle)
            except (OSError, RuntimeError) as error:
                close_errors.append(f"CloseHandle({label}) failed: {error}")
                unclosed.append((label, handle))
        if not close_errors:
            assert job_handle is not None
            return _WindowsProcessJob(job_handle, windows_api, process)
        failure = RuntimeError("; ".join(close_errors))
        resource_handles = unclosed

    rollback_errors = _rollback_windows_process_job_attach(
        process,
        process_handle,
        job_handle,
        assigned_to_job,
        resource_handles,
        cleanup_deadline,
        windows_api,
    )
    details = [str(failure), *rollback_errors]
    raise RuntimeError(
        "Windows Job Object attach failed: " + "; ".join(details)
    ) from failure


def _posix_process_snapshot() -> dict[int, ProcessIdentity]:
    try:
        snapshot = subprocess.run(
            ["ps", "-axo", "pid=,ppid=,lstart="],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=_CLEANUP_GRACE_SECONDS,
            check=True,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return {}
    processes: dict[int, ProcessIdentity] = {}
    for line in snapshot.splitlines():
        fields = line.split(None, 2)
        if len(fields) != 3:
            continue
        try:
            pid, parent_pid = map(int, fields[:2])
            session_id = os.getsid(pid)
        except (OSError, ValueError):
            continue
        processes[pid] = ProcessIdentity(
            pid=pid,
            parent_pid=parent_pid,
            session_id=session_id,
            start_identity=fields[2].strip(),
        )
    return processes


def _windows_process_state(
    pid: int,
) -> tuple[ProcessIdentity | None, bool, str | None]:
    try:
        import ctypes
        from ctypes import wintypes

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        open_process = kernel32.OpenProcess
        open_process.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        open_process.restype = wintypes.HANDLE
        get_process_times = kernel32.GetProcessTimes
        get_process_times.argtypes = [
            wintypes.HANDLE,
            ctypes.POINTER(wintypes.FILETIME),
            ctypes.POINTER(wintypes.FILETIME),
            ctypes.POINTER(wintypes.FILETIME),
            ctypes.POINTER(wintypes.FILETIME),
        ]
        get_process_times.restype = wintypes.BOOL
        close_handle = kernel32.CloseHandle
        close_handle.argtypes = [wintypes.HANDLE]
        close_handle.restype = wintypes.BOOL
        wait_for_single_object = kernel32.WaitForSingleObject
        wait_for_single_object.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        wait_for_single_object.restype = wintypes.DWORD
        process_query_limited_information = 0x1000
        synchronize = 0x00100000
        wait_object_0 = 0
        wait_timeout = 0x00000102
        wait_failed = 0xFFFFFFFF
        handle = open_process(
            process_query_limited_information | synchronize, False, pid
        )
        if not handle:
            error = ctypes.get_last_error()
            if error in {0, 87, 1168}:
                return None, False, None
            return None, True, f"OpenProcess({pid}) failed with Windows error {error}"
        creation = wintypes.FILETIME()
        exit_time = wintypes.FILETIME()
        kernel_time = wintypes.FILETIME()
        user_time = wintypes.FILETIME()
        try:
            if not get_process_times(
                handle,
                ctypes.byref(creation),
                ctypes.byref(exit_time),
                ctypes.byref(kernel_time),
                ctypes.byref(user_time),
            ):
                error = ctypes.get_last_error()
                return (
                    None,
                    True,
                    f"GetProcessTimes({pid}) failed with Windows error {error}",
                )
            wait_result = wait_for_single_object(handle, 0)
            if wait_result == wait_failed:
                error = ctypes.get_last_error()
                return (
                    None,
                    True,
                    f"WaitForSingleObject({pid}) failed with Windows error {error}",
                )
        finally:
            close_handle(handle)
        identity = ProcessIdentity(
            pid=pid,
            parent_pid=None,
            session_id=None,
            start_identity=f"{creation.dwHighDateTime:08x}{creation.dwLowDateTime:08x}",
        )
        if wait_result == wait_timeout:
            return identity, True, None
        if wait_result == wait_object_0:
            return identity, False, None
        return (
            identity,
            True,
            f"WaitForSingleObject({pid}) returned unexpected status {wait_result}",
        )
    except (AttributeError, ImportError, OSError) as error:
        return None, True, f"Windows process query is unavailable: {error}"


def _windows_process_identity(pid: int) -> ProcessIdentity | None:
    identity, _, _ = _windows_process_state(pid)
    return identity


def _current_process_identity(pid: int) -> ProcessIdentity | None:
    if pid <= 0:
        return None
    if os.name == "posix":
        return _posix_process_snapshot().get(pid)
    if os.name == "nt":
        return _windows_process_identity(pid)
    return None


def _process_is_running(pid: int) -> bool:
    if pid <= 0:
        return False
    if os.name == "posix":
        try:
            status = subprocess.run(
                ["ps", "-o", "stat=", "-p", str(pid)],
                capture_output=True,
                text=True,
                encoding="utf-8",
                timeout=_CLEANUP_GRACE_SECONDS,
                check=False,
            ).stdout.strip()
        except (OSError, subprocess.SubprocessError):
            status = ""
        return bool(status) and not status.startswith("Z")
    if os.name == "nt":
        _, running, _ = _windows_process_state(pid)
        return running
    try:
        os.kill(pid, 0)
    except OSError:
        return False
    return True


def _wait_for_process_pids(pids: set[int], timeout_seconds: float) -> None:
    deadline = time.monotonic() + timeout_seconds
    while any(_process_is_running(pid) for pid in pids):
        if time.monotonic() >= deadline:
            return
        time.sleep(0.01)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--binary-arg", action="append", default=[])
    parser.add_argument("--protocol-version", required=True)
    parser.add_argument(
        "--tasks-capability", required=True, choices=("on", "off")
    )
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--profile", choices=("native", "compatibility"))
    parser.add_argument("--target", choices=EVIDENCE_TARGETS)
    parser.add_argument("--timeout-seconds", type=float, default=20.0)
    args = parser.parse_args()
    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be positive")
    if (args.profile is None) != (args.target is None):
        parser.error("--profile and --target must be provided together")
    executable = Path(args.binary)
    command = [
        str(executable.resolve()) if executable.exists() else args.binary,
        *args.binary_arg,
    ]
    result = WireProbe(
        command,
        protocol_version=args.protocol_version,
        tasks_capability=args.tasks_capability,
        profile=args.profile,
        target=args.target,
        timeout_seconds=args.timeout_seconds,
    ).run()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
