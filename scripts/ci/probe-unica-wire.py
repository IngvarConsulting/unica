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
            _terminate_unregistered_process_tree(self.process)
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
            result = self.process.wait(timeout=self._remaining_timeout())
        except subprocess.TimeoutExpired as error:
            self.terminate_tree()
            raise SystemExit(
                f"{self.error_label} exceeded its aggregate deadline: {self._detail()}"
            ) from error
        remaining = max(0.0, self.deadline - time.monotonic())
        self.reader.join(timeout=remaining)
        self.error_reader.join(timeout=remaining)
        if self.reader.is_alive() or self.error_reader.is_alive():
            raise SystemExit(
                f"{self.error_label} reader threads did not stop before the aggregate deadline: "
                f"{self._detail()}"
            )
        detail = self._detail()
        self._close_streams()
        if result != 0:
            raise SystemExit(f"{self.error_label} exited with {result}: {detail}")

    def terminate_tree(self) -> None:
        _terminate_unregistered_process_tree(self.process)
        for reader in (self.reader, self.error_reader):
            reader.join(timeout=_CLEANUP_GRACE_SECONDS)
        self._close_streams()

    def _close_streams(self) -> None:
        for stream in (
            self.process.stdin,
            self.process.stdout,
            self.process.stderr,
        ):
            if stream is not None and not stream.closed:
                stream.close()


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


def _terminate_unregistered_process_tree(process: subprocess.Popen[str]) -> None:
    if os.name == "posix":
        owned_pids = _posix_owned_process_pids(
            process.pid,
            set(),
            public_running=process.poll() is None,
        )
        _signal_processes(owned_pids, signal.SIGTERM)
        try:
            process.wait(timeout=_CLEANUP_GRACE_SECONDS)
        except subprocess.TimeoutExpired:
            pass
        survivors = {pid for pid in owned_pids if _process_is_running(pid)}
        _signal_processes(survivors, signal.SIGKILL)
        try:
            process.wait(timeout=_CLEANUP_GRACE_SECONDS)
        except subprocess.TimeoutExpired:
            pass
        _wait_for_process_pids(owned_pids, _CLEANUP_GRACE_SECONDS)
        return
    if os.name == "nt":
        _taskkill_process_tree(process.pid)
    elif process.poll() is None:
        try:
            process.terminate()
        except OSError:
            pass
    try:
        process.wait(timeout=_CLEANUP_GRACE_SECONDS)
    except subprocess.TimeoutExpired:
        try:
            process.kill()
        except OSError:
            pass
        try:
            process.wait(timeout=_CLEANUP_GRACE_SECONDS)
        except subprocess.TimeoutExpired:
            pass


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


def _signal_processes(pids: set[int], signal_number: int) -> None:
    for pid in sorted(pids, reverse=True):
        try:
            os.kill(pid, signal_number)
        except OSError:
            pass


def _posix_owned_process_pids(
    public_pid: int,
    service_pids: set[int],
    *,
    public_running: bool,
) -> set[int]:
    try:
        snapshot = subprocess.run(
            ["ps", "-axo", "pid=,ppid="],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=_CLEANUP_GRACE_SECONDS,
            check=True,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        roots = set(service_pids)
        if public_running:
            roots.add(public_pid)
        return roots
    processes: dict[int, int] = {}
    for line in snapshot.splitlines():
        fields = line.split()
        if len(fields) != 2:
            continue
        try:
            pid, parent_pid = map(int, fields)
        except ValueError:
            continue
        processes[pid] = parent_pid
    owned = {pid for pid in service_pids if pid in processes}
    if public_running and public_pid in processes:
        owned.add(public_pid)
    while True:
        descendants = {
            pid for pid, parent_pid in processes.items() if parent_pid in owned
        }
        expanded = owned | descendants
        if expanded == owned:
            return owned
        owned = expanded


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
