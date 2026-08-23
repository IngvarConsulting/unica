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


class ProcessOwnership:
    """Tracks one spawned session and identities registered before escape.

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
    ) -> None:
        self.process = process
        self.public_identity = public_identity
        self.session_id = session_id

    @classmethod
    def capture(cls, process: subprocess.Popen[str]) -> ProcessOwnership:
        pid = getattr(process, "pid", 0)
        identity = _current_process_identity(pid)
        session_id = pid if os.name == "posix" and pid > 0 else None
        if identity is not None and identity.session_id is not None:
            session_id = identity.session_id
        return cls(process, identity, session_id)

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
    ) -> set[ProcessIdentity]:
        cleanup_deadline = time.monotonic() + max(0.0, timeout_seconds)
        survivors = self.survivors(identities)
        if survivors:
            if os.name == "nt":
                for identity in sorted(survivors, key=lambda item: item.pid):
                    if identity.identifies(_current_process_identity(identity.pid)):
                        _taskkill_process_tree(identity.pid)
            else:
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
        return self.wait(
            survivors, max(0.0, cleanup_deadline - time.monotonic())
        )

    def terminate(
        self,
        additional_identities: set[ProcessIdentity] | None = None,
        timeout_seconds: float = _CLEANUP_GRACE_SECONDS,
    ) -> set[ProcessIdentity]:
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
        elif os.name == "nt" and hasattr(subprocess, "CREATE_NEW_PROCESS_GROUP"):
            popen_options["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
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
        self.process_ownership = ProcessOwnership.capture(self.process)
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

    def _close_streams(self) -> None:
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
                except (OSError, ValueError):
                    pass


class WireProbe:
    def __init__(
        self,
        command: list[str],
        *,
        protocol_version: str,
        tasks_capability: str,
        timeout_seconds: float,
        environment: dict[str, str] | None = None,
        cwd: Path | None = None,
    ) -> None:
        self.command = command
        self.protocol_version = protocol_version
        self.tasks_capability = tasks_capability
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
                "protocolVersion": self.protocol_version,
                "responseKinds": session.response_kinds,
                "serverInfo": server_info,
                "serverProtocolVersion": server_protocol_version,
                "tasksCapability": self.tasks_capability,
                "toolCount": len(tool_names),
                "toolNames": sorted(tool_names),
            }
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


def _taskkill_process_tree(pid: int) -> None:
    try:
        subprocess.run(
            ["taskkill", "/PID", str(pid), "/T", "/F"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=_CLEANUP_GRACE_SECONDS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        pass


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


def _windows_process_identity(pid: int) -> ProcessIdentity | None:
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
        handle = open_process(0x1000, False, pid)
        if not handle:
            return None
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
                return None
        finally:
            close_handle(handle)
        return ProcessIdentity(
            pid=pid,
            parent_pid=None,
            session_id=None,
            start_identity=f"{creation.dwHighDateTime:08x}{creation.dwLowDateTime:08x}",
        )
    except (AttributeError, ImportError, OSError):
        return None


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
    parser.add_argument("--timeout-seconds", type=float, default=20.0)
    args = parser.parse_args()
    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be positive")
    executable = Path(args.binary)
    command = [
        str(executable.resolve()) if executable.exists() else args.binary,
        *args.binary_arg,
    ]
    result = WireProbe(
        command,
        protocol_version=args.protocol_version,
        tasks_capability=args.tasks_capability,
        timeout_seconds=args.timeout_seconds,
    ).run()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
