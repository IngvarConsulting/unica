"""LSP-драйвер приёмочного корпуса (#2180).

Сценарий профиля `lsp` идёт через `unica lsp` так же, как редактор: по stdio
с кадрами `Content-Length`, на собранном бинаре и над копией фикстуры. Шаг —
метод LSP (`"tool": "lsp:textDocument/documentSymbol"`) и его параметры в
`args`. Документ называется адресом тела модуля, как его читает MCP:
`"$document/main:CommonModule.Tools.Body"` превращается в
`unica://<пространство>/main/CommonModule.Tools.Body`, где пространство —
то, что сервер выдал папке в `initialize`; `"$file/<путь>"` — `file://`
настоящего файла рабочей папки. Ответ проверяется теми же
утверждениями по JSON Pointer, что и шаги MCP; `result` ответа лежит под
`/result`. Отказ демона замораживается его кодом (`lsp_document_unknown`).
"""

from __future__ import annotations

import json
import os
import queue
import re
import shutil
import subprocess
import tempfile
import threading
import time
from pathlib import Path
from urllib.parse import quote

from scripts.ci.prepared_binary import binary_path
from tests.ci.acceptance_controls import (
    capture_values,
    content_mismatches,
    matches_step,
    substitute_capture,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKSPACE = "tests/fixtures/acceptance/workspace-lsp"
DRIVER = "lsp-stdio"
PREFIX = "lsp:"
DOCUMENT = "$document/"
FILE = "$file/"
RESPONSE_TIMEOUT_SECONDS = 60.0
# Уведомления редактора: ответа нет, шаг считается `ok`, если сервер принял
# кадр и продолжает отвечать.
NOTIFICATIONS = {
    "textDocument/didOpen",
    "textDocument/didChange",
    "textDocument/didClose",
}


def failure_codes() -> set[str]:
    """Коды отказов двери, как их пишет `unica lsp` в `error.message`."""
    source = (REPO_ROOT / "crates/unica-coder/src/interfaces/lsp.rs").read_text(encoding="utf-8")
    return set(re.findall(r'V5LspFailure::\w+ => "(lsp_\w+)"', source))


def is_lsp_step(step) -> bool:
    return step["tool"].startswith(PREFIX)


def validate_scenario(scenario) -> None:
    if scenario.get("driver") != DRIVER or scenario.get("workspace") != WORKSPACE:
        raise ValueError(f"{scenario['id']}: an LSP scenario runs {DRIVER} on {WORKSPACE}")
    for index, step in enumerate(scenario["wire"]):
        if not is_lsp_step(step) or not step["tool"][len(PREFIX):]:
            raise ValueError(f"{scenario['id']}: an LSP scenario steps through LSP methods only")
        if not isinstance(step.get("args"), dict):
            raise ValueError(f"{scenario['id']}: LSP params are an object")
        if step["tool"][len(PREFIX):] in NOTIFICATIONS:
            # A notification has no answer: a request after it proves the
            # server took it.
            if step["expect"] != ["ok"] or set(step) - {"tool", "args", "expect"}:
                raise ValueError(f"{scenario['id']}: a notification step only expects ok")
            if index == len(scenario["wire"]) - 1:
                raise ValueError(f"{scenario['id']}: a notification is followed by a request")


def document_uri(address: str, space: str) -> str:
    """`unica://` документа по адресу тела модуля `<набор>:<адрес>`."""
    source_set, logical = address.split(":", 1)
    return f"unica://{quote(space, safe='')}/{quote(source_set, safe='')}/{quote(logical, safe='')}"


def substitute(value, context):
    if isinstance(value, str) and value.startswith(DOCUMENT):
        return document_uri(value[len(DOCUMENT):], context["space"])
    if isinstance(value, str) and value.startswith(FILE):
        # The real file of the scenario's workspace, as an editor names it.
        return Path(context["workspace"]).joinpath(*value[len(FILE):].split("/")).resolve().as_uri()
    if isinstance(value, str) and value.startswith("$capture."):
        return substitute_capture(value, context)
    if isinstance(value, list):
        return [substitute(item, context) for item in value]
    if isinstance(value, dict):
        return {key: substitute(item, context) for key, item in value.items()}
    return value


def classify(response) -> tuple[str, str]:
    if response is None:
        return "error", "no response"
    if "result" in response:
        return "ok", ""
    message = str((response.get("error") or {}).get("message", ""))
    if message.startswith("unica: "):
        return "refused", message[len("unica: "):]
    return "error", json.dumps(response, ensure_ascii=False)[:200]


class LspServer:
    """Одно соединение редактора с `unica lsp`."""

    def __init__(self, workspace: Path, state: Path, binary: Path | None = None, name: str = "unica-lsp"):
        env = dict(os.environ)
        env["UNICA_PROVIDER_STATE_DIR"] = str(state)
        env["UNICA_DAEMON_IDLE_GRACE_MS"] = "5000"
        self.workspace = workspace
        self.stderr_path = state / f"{name}-stderr.log"
        self.stderr_file = open(self.stderr_path, "wb")  # noqa: SIM115 - lives as long as the process
        self.process = subprocess.Popen(
            [str(binary or binary_path(REPO_ROOT)), "lsp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr_file,
            cwd=workspace,
            env=env,
        )
        self.next_id = 0
        self.messages: "queue.Queue[dict | None]" = queue.Queue()
        self.reader = threading.Thread(target=self._pump, daemon=True)
        self.reader.start()

    def _pump(self) -> None:
        try:
            self._read_frames()
        finally:
            # A broken frame ends the stream for the waiting request too.
            self.messages.put(None)

    def _read_frames(self) -> None:
        stream = self.process.stdout
        assert stream is not None
        while True:
            length = None
            while True:
                line = stream.readline()
                if not line:
                    return
                line = line.strip()
                if not line:
                    break
                name, _, value = line.decode("ascii").partition(":")
                if name.lower() == "content-length":
                    length = int(value.strip())
            if length is None:
                return
            self.messages.put(json.loads(stream.read(length)))

    def send(self, payload) -> None:
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.process.stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii") + body)
        self.process.stdin.flush()

    def stderr_tail(self, limit: int = 1200) -> str:
        try:
            self.stderr_file.flush()
            return self.stderr_path.read_text(encoding="utf-8", errors="replace")[-limit:]
        except OSError:
            return ""

    def request(self, method: str, params, timeout: float = RESPONSE_TIMEOUT_SECONDS):
        self.next_id += 1
        request_id = self.next_id
        self.send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            try:
                if remaining <= 0:
                    raise queue.Empty
                message = self.messages.get(timeout=remaining)
            except queue.Empty:
                raise TimeoutError(
                    f"no answer to {method} within {timeout:.0f}s; stderr tail: {self.stderr_tail()}"
                ) from None
            if message is None:
                return None
            if message.get("method") == "window/logMessage" and (message.get("params") or {}).get("type") == 1:
                # The door reports a notification it refused as an error
                # message: the step must not pass as accepted.
                return {"error": {"message": "unica: logMessage " + str(message["params"].get("message"))}}
            if message.get("id") == request_id and "method" not in message:
                return message

    def notify(self, method: str, params) -> None:
        self.send({"jsonrpc": "2.0", "method": method, "params": params})

    def initialize(self) -> dict:
        folder = self.workspace.resolve().as_uri()
        answer = self.request("initialize", {
            "processId": os.getpid(),
            "rootUri": folder,
            "capabilities": {},
            "workspaceFolders": [{"uri": folder, "name": self.workspace.name}],
        })
        if not answer or "result" not in answer:
            raise RuntimeError(f"initialize failed: {answer}; stderr tail: {self.stderr_tail()}")
        self.notify("initialized", {})
        return answer["result"]

    def close(self) -> None:
        try:
            if self.process.poll() is None:
                self.request("shutdown", None, timeout=10)
                self.notify("exit", None)
                self.process.wait(timeout=10)
        except (OSError, TimeoutError, subprocess.TimeoutExpired):
            pass
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait()
        self.reader.join(timeout=10)
        for stream in (self.process.stdin, self.process.stdout, self.stderr_file):
            try:
                stream.close()
            except OSError:
                pass


def granted_space(initialized: dict) -> str:
    workspaces = initialized["capabilities"]["experimental"]["unica"]["workspaces"]
    spaces = [entry["space"] for entry in workspaces if "space" in entry]
    if len(spaces) != 1:
        raise RuntimeError(f"expected one granted space, got {workspaces}")
    return spaces[0]


def run_scenario(scenario, root: Path, binary: Path | None = None) -> list[str]:
    """Прогнать один сценарий; вернуть расхождения с замороженным ответом."""
    workspace = root / "workspace"
    shutil.copytree(REPO_ROOT / WORKSPACE, workspace)
    state = root / "state"
    state.mkdir()
    failures: list[str] = []
    try:
        server = LspServer(workspace, state, binary)
    except OSError as error:
        return [f"{scenario['id']}: unica lsp did not start: {error}"]
    try:
        context = {"captures": {}, "workspace": workspace, "space": granted_space(server.initialize())}
        for index, step in enumerate(scenario["wire"]):
            method = step["tool"][len(PREFIX):]
            label = f"{scenario['id']} step {index} {method}"
            params = substitute(step["args"], context)
            if method in NOTIFICATIONS:
                server.notify(method, params)
                actual, note, structured = "ok", "", {}
            else:
                response = server.request(method, params)
                actual, note = classify(response)
                structured = {"result": (response or {}).get("result")}
            if not matches_step(step, actual):
                failures.append(f"{label}: expected {step['expect']}, got {actual} ({note})")
                break
            found = content_mismatches(step, structured, actual, note, workspace, context)
            if found:
                failures.extend(f"{label}: {failure}" for failure in found)
                break
            context["captures"] = capture_values(step, structured, workspace, context)
    except (OSError, RuntimeError, TimeoutError, ValueError, KeyError) as error:
        # One scenario's failure is reported with the others, not instead.
        failures.append(f"{scenario['id']}: {type(error).__name__}: {error}")
    finally:
        server.close()
    return failures


def run_lsp_corpus(test_case, scenarios, binary: Path | None = None) -> None:
    if not scenarios:
        raise ValueError("the LSP profile is empty")
    failures: list[str] = []
    for scenario in scenarios:
        validate_scenario(scenario)
        # The daemon outlives the door by its idle grace and still holds
        # its state on Windows when the directory is removed.
        with tempfile.TemporaryDirectory(prefix="unica-lsp-acceptance-", ignore_cleanup_errors=True) as raw:
            failures.extend(run_scenario(scenario, Path(raw).resolve(), binary))
    test_case.assertEqual(failures, [], "LSP scenarios differ from their frozen answers")
