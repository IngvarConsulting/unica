#!/usr/bin/env python3
"""Наблюдатель адресов: дерево символических адресов Unica в браузере.

Запускает `unica` как MCP-сервер по stdio на рабочей области и поднимает на
127.0.0.1 страницу с деревом адресов, словарём `can` выбранного узла,
контрактом операции и предпросмотром `apply`. Наблюдатель — обычный
MCP-клиент: видит ровно то, что видит агент на проводе (issue #1455).

Рабочую область сервер узнаёт из первого корня клиента (`roots/list`), а без
него — из каталога запуска. Наблюдатель объявляет `roots`, отвечает корнем
рабочей области и запускает сервер в ней же; переменные проекта хоста
(`CLAUDE_PROJECT_DIR`, `ZCODE_PROJECT_DIR`) из окружения сервера убирает,
чтобы наблюдатель, запущенный из сессии агента, не смотрел в чужой проект.

Исполнение плана не предлагается: API принимает только `{at, ops}` и
отклоняет любой запрос с `executionToken`, поэтому файлы не меняются.

Запуск:
  python3.12 scripts/dev/unica-observer.py --workspace <каталог>
      [--unica target/release/unica] [--port 0] [--no-open]

Первой строкой stdout печатается `UNICA_OBSERVER_URL=<адрес страницы>`.
"""
from __future__ import annotations

import argparse
import collections
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import webbrowser
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
PROTOCOL_VERSION = "2025-06-18"
# Переменные, через которые хост называет свой проект: у наблюдателя проект
# один — `--workspace`.
HOST_PROJECT_VARIABLES = ("CLAUDE_PROJECT_DIR", "ZCODE_PROJECT_DIR")
REQUEST_TIMEOUT_SECONDS = 300.0
LOG_CAPACITY = 2000
TOKEN_FIELD = "executionToken"
MAX_BODY_BYTES = 4 * 1024 * 1024


def default_binary() -> Path:
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO_ROOT / "target"))
    if not target.is_absolute():
        target = REPO_ROOT / target
    return target / "release" / ("unica.exe" if os.name == "nt" else "unica")


class McpError(RuntimeError):
    pass


class _Pending:
    __slots__ = ("event", "response", "started")

    def __init__(self) -> None:
        self.event = threading.Event()
        self.response: dict[str, Any] | None = None
        self.started = time.monotonic()


class McpClient:
    """Одна сессия JSON-RPC по stdio: один поток чтения, ответы по `id`.

    Запросы сервера (`roots/list`, `ping`) поток чтения обслуживает сам, не
    дожидаясь ответа на исходящий вызов, — иначе сервер, который спрашивает
    корни посреди `tools/call`, и клиент, который ждёт этот вызов, ждали бы
    друг друга.
    """

    def __init__(
        self,
        binary: Path,
        workspace: Path,
        env: dict[str, str] | None = None,
        timeout: float = REQUEST_TIMEOUT_SECONDS,
    ) -> None:
        self.workspace = workspace
        self.timeout = timeout
        # Первый вызов инструмента после старта наблюдателя — холодный: демон
        # и кеши ещё не прогреты. Время каждого вызова меряется по стене.
        self.tool_calls = 0
        self.tool_calls_lock = threading.Lock()
        self.root_uri = workspace.as_uri()
        self.log: collections.deque[dict[str, Any]] = collections.deque(maxlen=LOG_CAPACITY)
        self.log_seq = 0
        self.log_lock = threading.Lock()
        self.write_lock = threading.Lock()
        self.pending: dict[int, _Pending] = {}
        self.pending_lock = threading.Lock()
        self.next_id = 0
        self.closed = threading.Event()
        child_env = dict(os.environ if env is None else env)
        for name in HOST_PROJECT_VARIABLES:
            child_env.pop(name, None)
        self.stderr_file = tempfile.TemporaryFile()
        self.process = subprocess.Popen(
            [str(binary)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr_file,
            cwd=workspace,
            env=child_env,
        )
        self.reader = threading.Thread(target=self._read, name="mcp-reader", daemon=True)
        self.reader.start()
        self.initialize_result = self.request(
            "initialize",
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"roots": {"listChanged": False}},
                "clientInfo": {"name": "unica-observer", "version": "0"},
            },
        )
        self.notify("notifications/initialized")
        self.tools = self.request("tools/list", {}).get("tools") or []

    # --- журнал ---------------------------------------------------------
    def _record(self, direction: str, payload: dict[str, Any], **extra: Any) -> None:
        with self.log_lock:
            self.log_seq += 1
            self.log.append(
                {"seq": self.log_seq, "time": time.time(), "dir": direction, "payload": payload, **extra}
            )

    def log_since(self, seq: int) -> list[dict[str, Any]]:
        with self.log_lock:
            return [entry for entry in self.log if entry["seq"] > seq]

    # --- транспорт ------------------------------------------------------
    def _send(self, payload: dict[str, Any]) -> None:
        line = (json.dumps(payload, ensure_ascii=False) + "\n").encode()
        with self.write_lock:
            if self.process.stdin is None or self.process.poll() is not None:
                raise McpError("unica завершился; " + self.stderr_tail())
            try:
                self.process.stdin.write(line)
                self.process.stdin.flush()
            except (OSError, ValueError) as error:
                # Труба закрыта (BrokenPipe) или поток уже закрыт: отказ, а не обрыв.
                raise McpError(f"unica не принимает запросы: {error}; " + self.stderr_tail()) from error
        self._record("out", payload)

    def _read(self) -> None:
        stream = self.process.stdout
        assert stream is not None
        for raw in iter(stream.readline, b""):
            raw = raw.strip()
            if not raw:
                continue
            try:
                message = json.loads(raw)
            except json.JSONDecodeError:
                self._record("in", {"unparsed": raw.decode("utf-8", "replace")})
                continue
            if not isinstance(message, dict):
                self._record("in", message)
                continue
            if "method" in message and "id" in message:
                self._record("in", message)
                self._answer_server_request(message)
            elif "method" not in message and "id" in message:
                with self.pending_lock:
                    waiter = self.pending.pop(message["id"], None)
                elapsed = None if waiter is None else round((time.monotonic() - waiter.started) * 1000)
                self._record("in", message, elapsedMs=elapsed)
                if waiter is not None:
                    waiter.response = message
                    waiter.event.set()
            else:
                # Уведомления (progress, logging) только журналируются.
                self._record("in", message)
        self.closed.set()
        with self.pending_lock:
            waiters, self.pending = list(self.pending.values()), {}
        for waiter in waiters:
            waiter.event.set()

    def _answer_server_request(self, message: dict[str, Any]) -> None:
        method = message["method"]
        if method == "roots/list":
            reply = {"result": {"roots": [{"uri": self.root_uri, "name": self.workspace.name}]}}
        elif method == "ping":
            reply = {"result": {}}
        else:
            reply = {"error": {"code": -32601, "message": f"observer does not serve {method}"}}
        try:
            self._send({"jsonrpc": "2.0", "id": message["id"], **reply})
        except (McpError, OSError):
            pass

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        payload: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            payload["params"] = params
        self._send(payload)

    def request(self, method: str, params: dict[str, Any], timeout: float | None = None) -> dict[str, Any]:
        timeout = self.timeout if timeout is None else timeout
        waiter = _Pending()
        with self.pending_lock:
            self.next_id += 1
            request_id = self.next_id
            self.pending[request_id] = waiter
        if self.closed.is_set():
            # Поток чтения уже вышел: ответа не будет, ждать срок незачем.
            with self.pending_lock:
                self.pending.pop(request_id, None)
            raise McpError(f"{method}: unica закрыл поток; " + self.stderr_tail())
        if method == "tools/call":
            # Токен прогресса: долгий вызов виден в журнале уведомлениями.
            params = {**params, "_meta": {"progressToken": f"observer-{request_id}"}}
        waiter.started = time.monotonic()
        self._send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        if not waiter.event.wait(timeout):
            with self.pending_lock:
                self.pending.pop(request_id, None)
            try:
                self.notify("notifications/cancelled", {"requestId": request_id, "reason": "observer timeout"})
            except McpError:
                pass
            raise McpError(f"{method}: нет ответа за {timeout:.0f} с")
        response = waiter.response
        if response is None:
            raise McpError(f"{method}: unica закрыл поток; " + self.stderr_tail())
        if "error" in response:
            raise McpError(f"{method}: {json.dumps(response['error'], ensure_ascii=False)}")
        result = response.get("result") or {}
        if not isinstance(result, dict):
            raise McpError(f"{method}: неожиданная форма ответа: {json.dumps(result, ensure_ascii=False)[:300]}")
        return result

    def stderr_tail(self, limit: int = 2000) -> str:
        try:
            self.stderr_file.flush()
            self.stderr_file.seek(0)
            return self.stderr_file.read().decode("utf-8", "replace")[-limit:]
        except (OSError, ValueError):
            return ""

    def close(self) -> None:
        try:
            if self.process.stdin is not None:
                self.process.stdin.close()
        except OSError:
            pass
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.reader.join(timeout=5)
        if self.process.stdout is not None:
            self.process.stdout.close()
        self.stderr_file.close()

    # --- инструменты ------------------------------------------------------
    def tool_name(self, verb: str) -> str:
        """Имя инструмента на проводе: `unica.view` или `view`, как объявил сервер."""
        names = [tool.get("name", "") for tool in self.tools]
        for name in names:
            if name == verb or name.endswith("." + verb):
                return name
        raise McpError(f"сервер не объявил инструмент {verb}; объявлены: {', '.join(names)}")

    def _call_once(self, name: str, arguments: dict[str, Any]) -> tuple[bool, dict[str, Any]]:
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        structured = result.get("structuredContent")
        if structured is None:
            # Без structuredContent ответ доступен только текстом.
            parts = result.get("content") or []
            if not isinstance(parts, list):
                raise McpError(f"{name}: content не список: {json.dumps(parts, ensure_ascii=False)[:300]}")
            texts = [
                part["text"]
                for part in parts
                if isinstance(part, dict) and part.get("type") == "text" and isinstance(part.get("text"), str)
            ]
            try:
                structured = json.loads("".join(texts))
            except json.JSONDecodeError:
                structured = {"ok": False, "summary": "".join(texts)}
        if not isinstance(structured, dict):
            raise McpError(f"{name}: structuredContent не объект: {json.dumps(structured, ensure_ascii=False)[:300]}")
        return bool(result.get("isError")), structured

    def call_tool(self, verb: str, arguments: dict[str, Any]) -> dict[str, Any]:
        """Вызов инструмента со временем по стене.

        Долгий вызов сервер передаёт в задачу (`data.task`, `next` на
        `task.result`) — наблюдатель дожидается её, как агент, и отдаёт
        итоговый ответ; время включает ожидание задачи.
        """
        name = self.tool_name(verb)
        with self.tool_calls_lock:
            self.tool_calls += 1
            cold = self.tool_calls == 1
        started = time.monotonic()
        is_error, structured = self._call_once(name, arguments)
        handoff_ms = None
        polls = 0
        while True:
            data = structured.get("data")
            task = data.get("task") if isinstance(data, dict) else None
            # Незавершённые состояния задачи v0.13 — `queued` и `working`
            # (`task_tools.rs`); остальные терминальны.
            if not isinstance(task, dict) or task.get("status") not in ("queued", "working"):
                break
            task_id = task.get("taskId")
            if not isinstance(task_id, str) or not task_id:
                raise McpError(f"{name}: задача без taskId: {json.dumps(task, ensure_ascii=False)[:300]}")
            if handoff_ms is None:
                handoff_ms = round((time.monotonic() - started) * 1000)
            if time.monotonic() - started > self.timeout:
                try:
                    self._call_once(self.tool_name("task.cancel"), {"taskId": task_id})
                except McpError:
                    pass
                raise McpError(f"{name}: задача {task_id} не завершилась за {self.timeout:.0f} с и отменена")
            polls += 1
            is_error, structured = self._call_once(
                self.tool_name("task.result"), {"taskId": task_id, "waitMs": 5000}
            )
        answer = {
            "isError": is_error,
            "elapsedMs": round((time.monotonic() - started) * 1000),
            "cold": cold,
            "result": structured,
        }
        if handoff_ms is not None:
            answer["task"] = {"handoffMs": handoff_ms, "polls": polls}
        return answer


def contains_token(value: Any) -> bool:
    if isinstance(value, dict):
        return TOKEN_FIELD in value or any(contains_token(item) for item in value.values())
    if isinstance(value, list):
        return any(contains_token(item) for item in value)
    return False


class ApiError(Exception):
    def __init__(self, status: HTTPStatus, message: str) -> None:
        super().__init__(message)
        self.status = status


def view_arguments(body: dict[str, Any]) -> dict[str, Any]:
    """Аргументы `view`: `at`, `cursor` и прочие поля из `extra` (например, `limit`).

    `extra` не переопределяет `at` и `cursor` и не несёт `executionToken`.
    """
    if contains_token(body):
        raise ApiError(HTTPStatus.FORBIDDEN, "observer never sends executionToken")
    arguments: dict[str, Any] = {}
    at = body.get("at")
    if at not in (None, ""):
        if not isinstance(at, str):
            raise ApiError(HTTPStatus.BAD_REQUEST, "at must be a string")
        arguments["at"] = at
    cursor = body.get("cursor")
    if cursor:
        if not isinstance(cursor, str):
            raise ApiError(HTTPStatus.BAD_REQUEST, "cursor must be a string")
        arguments["cursor"] = cursor
    extra = body.get("extra") or {}
    if not isinstance(extra, dict):
        raise ApiError(HTTPStatus.BAD_REQUEST, "extra must be an object")
    reserved = {"at", "cursor"} & set(extra)
    if reserved:
        raise ApiError(HTTPStatus.BAD_REQUEST, f"extra must not set {sorted(reserved)}; pass them at the top level")
    unknown = set(body) - {"at", "cursor", "extra"}
    if unknown:
        raise ApiError(HTTPStatus.BAD_REQUEST, f"view accepts only at, cursor and extra, got {sorted(unknown)}")
    arguments.update(extra)
    return arguments


def preview_arguments(body: dict[str, Any]) -> dict[str, Any]:
    """Только форма планирования `{at, ops}`; исполнение плана не пропускается."""
    if contains_token(body):
        raise ApiError(HTTPStatus.FORBIDDEN, "observer never executes a plan: executionToken is refused")
    unknown = set(body) - {"at", "ops"}
    if unknown:
        raise ApiError(HTTPStatus.BAD_REQUEST, f"preview accepts only at and ops, got {sorted(unknown)}")
    if not isinstance(body.get("at"), str) or not isinstance(body.get("ops"), list):
        raise ApiError(HTTPStatus.BAD_REQUEST, "preview needs a string at and an ops list")
    return {"at": body["at"], "ops": body["ops"]}


class Observer:
    def __init__(self, client: McpClient) -> None:
        self.client = client

    def info(self) -> dict[str, Any]:
        return {
            "workspace": str(self.client.workspace),
            "rootUri": self.client.root_uri,
            "server": self.client.initialize_result.get("serverInfo"),
            "protocolVersion": self.client.initialize_result.get("protocolVersion"),
            "instructions": self.client.initialize_result.get("instructions"),
            "tools": [tool.get("name") for tool in self.client.tools],
        }

    def handle(self, method: str, path: str, body: dict[str, Any]) -> dict[str, Any]:
        if method == "GET" and path == "/api/info":
            return self.info()
        if method == "GET" and path.startswith("/api/log"):
            since = 0
            if "since=" in path:
                try:
                    since = int(path.split("since=", 1)[1].split("&", 1)[0])
                except ValueError:
                    since = 0
            return {"entries": self.client.log_since(since)}
        if method == "POST" and path == "/api/view":
            arguments = view_arguments(body)
            return {"arguments": arguments, **self.client.call_tool("view", arguments)}
        if method == "POST" and path == "/api/apply":
            arguments = preview_arguments(body)
            return {"arguments": arguments, **self.client.call_tool("apply", arguments)}
        raise ApiError(HTTPStatus.NOT_FOUND, f"no route {method} {path}")


def make_handler(observer: Observer, page: bytes):
    class Handler(BaseHTTPRequestHandler):
        server_version = "unica-observer"

        def log_message(self, format: str, *args: Any) -> None:  # noqa: A002 - stdlib signature
            pass

        def _host_allowed(self) -> bool:
            # Защита от DNS rebinding: страница отвечает только своему адресу,
            # а запрос с чужой страницы (заголовок Origin) отвергается.
            port = self.server.server_address[1]
            own = {f"127.0.0.1:{port}", f"localhost:{port}"}
            if self.headers.get("Host") not in own:
                return False
            origin = self.headers.get("Origin")
            return origin is None or origin in {f"http://{host}" for host in own}

        def _reply(self, status: HTTPStatus, payload: Any, content_type: str = "application/json") -> None:
            data = payload if isinstance(payload, bytes) else json.dumps(payload, ensure_ascii=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", content_type + ("; charset=utf-8" if "charset" not in content_type else ""))
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(data)

        def _dispatch(self, method: str) -> None:
            if not self._host_allowed():
                self._reply(HTTPStatus.FORBIDDEN, {"error": "unexpected Host or Origin header"})
                return
            if method == "GET" and self.path in ("/", "/index.html"):
                self._reply(HTTPStatus.OK, page, "text/html")
                return
            body: dict[str, Any] = {}
            if method == "POST":
                # Только JSON: простая форма с чужой страницы так не отправится.
                if not (self.headers.get("Content-Type") or "").startswith("application/json"):
                    self._reply(HTTPStatus.UNSUPPORTED_MEDIA_TYPE, {"error": "application/json required"})
                    return
                raw_length = self.headers.get("Content-Length") or "0"
                if not raw_length.isdigit() or int(raw_length) > MAX_BODY_BYTES:
                    self._reply(HTTPStatus.BAD_REQUEST, {"error": f"bad Content-Length: {raw_length!r}"})
                    return
                try:
                    body = json.loads(self.rfile.read(int(raw_length)) or b"{}")
                except (json.JSONDecodeError, UnicodeDecodeError) as error:
                    self._reply(HTTPStatus.BAD_REQUEST, {"error": f"invalid JSON: {error}"})
                    return
                if not isinstance(body, dict):
                    self._reply(HTTPStatus.BAD_REQUEST, {"error": "body must be a JSON object"})
                    return
            try:
                self._reply(HTTPStatus.OK, observer.handle(method, self.path, body))
            except ApiError as error:
                self._reply(error.status, {"error": str(error)})
            except McpError as error:
                self._reply(HTTPStatus.BAD_GATEWAY, {"error": str(error)})
            except Exception as error:  # noqa: BLE001 - ответ вместо оборванного соединения
                self._reply(HTTPStatus.BAD_GATEWAY, {"error": f"{type(error).__name__}: {error}"})

        def do_GET(self) -> None:  # noqa: N802 - stdlib name
            self._dispatch("GET")

        def do_POST(self) -> None:  # noqa: N802 - stdlib name
            self._dispatch("POST")

    return Handler


PAGE = r"""<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Наблюдатель адресов</title>
<style>
:root {
  --bg: #fbfbfa; --panel: #ffffff; --text: #1d1d1b; --muted: #6b6b66; --line: #e3e2dd;
  --accent: #2f5fb3; --accent-bg: #e8eefa; --ok: #1f7a3f; --bad: #b3261e; --warn: #8a5a00;
  --code: #f3f2ee; --sel: #fff2c6;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #161615; --panel: #1f1f1d; --text: #e9e8e3; --muted: #9a998f; --line: #34332f;
    --accent: #8fb0ff; --accent-bg: #23304a; --ok: #6fcf8f; --bad: #ff8a80; --warn: #e6b450;
    --code: #262623; --sel: #4a3f17;
  }
}
* { box-sizing: border-box; }
body { margin: 0; display: flex; flex-direction: column; height: 100vh; background: var(--bg); color: var(--text); font: 14px/1.45 system-ui, -apple-system, "Segoe UI", sans-serif; }
header { display: flex; gap: 8px; align-items: center; padding: 10px 16px; border-bottom: 1px solid var(--line); background: var(--panel); flex-wrap: wrap; }
header h1 { font-size: 15px; margin: 0 12px 0 0; }
header .ws { color: var(--muted); font-size: 12px; overflow-wrap: anywhere; }
#addr { flex: 1 1 320px; min-width: 0; font: 13px ui-monospace, SFMono-Regular, Menlo, monospace; padding: 6px 8px; border: 1px solid var(--line); border-radius: 6px; background: var(--bg); color: var(--text); }
a { color: var(--accent); }
button { font: inherit; font-size: 13px; padding: 5px 10px; border: 1px solid var(--line); border-radius: 6px; background: var(--panel); color: var(--text); cursor: pointer; }
button:hover { border-color: var(--accent); }
button.primary { background: var(--accent); color: var(--panel); border-color: var(--accent); }
main { display: grid; grid-template-columns: var(--tree-w, 38%) 6px minmax(0, 1fr); flex: 1 1 auto; min-height: 0; }
.split { background: var(--line); touch-action: none; user-select: none; }
.split:hover, .split.drag { background: var(--accent); }
.split-v { cursor: col-resize; }
.split-h { flex: 0 0 6px; cursor: row-resize; }
@media (max-width: 760px) {
  body { height: auto; }
  main { grid-template-columns: 1fr; }
  #tree { max-height: 50vh; }
  .split { display: none; }
}
#tree, #node { overflow: auto; padding: 10px 12px; }
#tree { font: 13px ui-monospace, SFMono-Regular, Menlo, monospace; }
.row { display: flex; gap: 6px; align-items: baseline; padding: 1px 4px; border-radius: 4px; cursor: pointer; white-space: nowrap; }
.row:hover { background: var(--accent-bg); }
.row.sel { background: var(--sel); }
.tog { width: 14px; color: var(--muted); flex: none; text-align: center; }
.kids { margin-left: 14px; border-left: 1px dotted var(--line); padding-left: 4px; }
.cnt { color: var(--muted); font-size: 11px; }
.badge { font-size: 10px; padding: 0 5px; border-radius: 8px; border: 1px solid var(--line); color: var(--muted); }
.badge.can { color: var(--ok); border-color: var(--ok); }
.badge.graph { color: var(--warn); border-color: var(--warn); }
.err { color: var(--bad); }
.more { color: var(--accent); }
#node h2 { font-size: 15px; margin: 4px 0 8px; overflow-wrap: anywhere; }
.at { font: 13px ui-monospace, SFMono-Regular, Menlo, monospace; overflow-wrap: anywhere; }
dl { display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; margin: 8px 0; }
dt { color: var(--muted); }
dd { margin: 0; min-width: 0; }
pre { background: var(--code); padding: 8px; border-radius: 6px; overflow: auto; font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; margin: 4px 0; max-height: 360px; }
table { border-collapse: collapse; width: 100%; font-size: 13px; }
td, th { border-bottom: 1px solid var(--line); padding: 4px 6px; text-align: left; vertical-align: top; }
td code { font-size: 12px; }
.yes { color: var(--ok); } .no { color: var(--muted); }
textarea { width: 100%; min-height: 140px; font: 12px ui-monospace, SFMono-Regular, Menlo, monospace; background: var(--code); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 8px; }
section { margin: 14px 0; }
section h3 { font-size: 13px; text-transform: uppercase; letter-spacing: .04em; color: var(--muted); margin: 0 0 6px; }
#log { flex: 0 0 var(--log-h, 34vh); overflow: auto; border-top: 1px solid var(--line); background: var(--panel); padding: 6px 16px; font: 12px ui-monospace, SFMono-Regular, Menlo, monospace; }
#log details { border-bottom: 1px solid var(--line); padding: 2px 0; }
#log summary { cursor: pointer; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.dir-out { color: var(--accent); } .dir-in { color: var(--ok); }
</style>
</head>
<body>
<header>
  <h1>Наблюдатель адресов</h1>
  <input id="addr" placeholder="адрес, например main:Catalog.Валюты; пусто — корень" spellcheck="false">
  <button class="primary" id="go">Открыть</button>
  <button id="copy" title="Скопировать адрес выбранного узла">Копировать адрес</button>
  <span class="ws" id="ws"></span>
</header>
<main>
  <div id="tree"></div>
  <div class="split split-v" id="splitTree" title="Тяните, чтобы изменить ширину; двойной щелчок — по умолчанию"></div>
  <div id="node"><p class="cnt">Выберите узел в дереве или введите адрес.</p></div>
</main>
<div class="split split-h" id="splitLog" title="Тяните, чтобы изменить высоту журнала; двойной щелчок — по умолчанию"></div>
<div id="log"></div>
<script>
"use strict";
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
const pretty = (v) => JSON.stringify(v, null, 2);
const cache = new Map();
const PAGE_LIMIT = 50;
let selected = null;
let selectedRow = null;

async function api(path, body) {
  const init = body === undefined ? {} : {method: "POST", headers: {"Content-Type": "application/json"}, body: JSON.stringify(body)};
  const response = await fetch(path, init);
  const payload = await response.json();
  if (!response.ok) throw new Error(payload.error || response.statusText);
  return payload;
}

// Кеш хранит обещание: раскрытие и выбор одного узла делят один вызов,
// а не гоняют два одинаковых (корень на большой базе — десятки секунд).
async function view(at, cursor, fresh) {
  const key = (at || "") + "\u0000" + (cursor || "");
  const entry = cache.get(key);
  if (!fresh && entry) {
    const answer = await entry.pending;
    // «Из кеша» — только ответ, полученный раньше этого запроса.
    return entry.settledBefore ? {...answer, cached: true} : answer;
  }
  // Страница коллекции — наибольшая из допустимых (50); без `at` предел не принимается.
  const pending = api("/api/view", at ? {at, cursor, extra: {limit: PAGE_LIMIT}} : {});
  const record = {pending, settledBefore: false};
  cache.set(key, record);
  try {
    const answer = await pending;
    setTimeout(() => { record.settledBefore = true; });
    return answer;
  } catch (error) { cache.delete(key); throw error; }
}

// Ветви графа вызовов — коллекция анализатора, а не узлы набора исходников:
// словаря `can` у них нет (так же их обходит `walk()` в тестах честности `can`).
const isGraph = (at) => /\.(Caller|Callee)$/.test(at || "");

// Скелет аргументов по ключу словаря `can`, как его строит реестр операций.
const SKELETON = {from: "", items: [], values: {}, text: "", at: ""};
const skeleton = (key) => typeof key === "string" ? {[key]: SKELETON[key] ?? {}} : (key || {});
const timing = (answer) => answer && answer.elapsedMs !== undefined
  ? `<span class="cnt">${(answer.elapsedMs / 1000).toFixed(2)} с${answer.cold ? " · холодный" : ""}${answer.task ? ` · через задачу (передан через ${(answer.task.handoffMs / 1000).toFixed(1)} с)` : ""}${answer.cached ? " · из кеша страницы" : ""}</span>` : "";

function ticking(element, label) {
  const started = performance.now();
  element.textContent = label;
  const timer = setInterval(() => { element.textContent = `${label} ${((performance.now() - started) / 1000).toFixed(0)} с`; }, 1000);
  return () => clearInterval(timer);
}

function makeRow(label, at, meta) {
  const wrap = document.createElement("div");
  const row = document.createElement("div");
  row.className = "row";
  const tog = document.createElement("span");
  tog.className = "tog";
  tog.textContent = at !== undefined ? "▸" : "·";
  row.appendChild(tog);
  const name = document.createElement("span");
  name.textContent = label;
  row.appendChild(name);
  if (meta) row.insertAdjacentHTML("beforeend", meta);
  const kids = document.createElement("div");
  kids.className = "kids";
  kids.hidden = true;
  wrap.append(row, kids);
  if (at !== undefined) {
    let loaded = false;
    tog.onclick = async (event) => {
      event.stopPropagation();
      if (!loaded) {
        loaded = true;
        try { await fill(kids, at); }
        catch (error) { kids.insertAdjacentHTML("beforeend", `<div class="err">ошибка страницы: ${esc(error.message)}</div>`); }
      }
      kids.hidden = !kids.hidden;
      tog.textContent = kids.hidden ? "▸" : "▾";
    };
    row.onclick = () => select(at, row);
  }
  return wrap;
}

function shortLabel(at, parent) {
  if (parent && at.startsWith(parent + ".")) return at.slice(parent.length + 1);
  return at;
}

async function fill(container, at, cursor) {
  const loading = document.createElement("div");
  loading.className = "cnt";
  const stop = ticking(loading, "загрузка…");
  container.appendChild(loading);
  let answer;
  try { answer = await view(at, cursor); } catch (error) { stop(); loading.className = "err"; loading.textContent = error.message; return; }
  stop();
  loading.remove();
  const result = answer.result || {};
  const data = result.data || {};
  if (!result.ok) {
    const div = document.createElement("div");
    div.className = "err";
    div.textContent = "отказ: " + (result.summary || "");
    container.appendChild(div);
    return;
  }
  const parent = data.at || at || "";
  // Корень (`view {}`) — сведения о рабочей области, а не узел: ветвей у него
  // нет, детьми служат корни наборов исходников `<имя>:Configuration`.
  for (const set of (at ? [] : data.sourceSets || [])) {
    const rootAt = `${set.name}:Configuration`;
    container.appendChild(makeRow(rootAt, rootAt, ` <span class="badge">${esc(set.kind || "")}</span> <span class="cnt">${esc(set.sourceState || "")}</span>`));
  }
  for (const branch of data.branches || []) {
    const badges = `<span class="cnt">${branch.count ?? ""}</span>` + (isGraph(branch.at) ? ' <span class="badge graph">граф вызовов</span>' : "");
    container.appendChild(makeRow(shortLabel(branch.at, parent), branch.at, " " + badges));
  }
  for (const item of data.items || []) {
    if (item && typeof item.at === "string") {
      const meta = item.kind ? ` <span class="badge">${esc(item.kind)}</span>` : "";
      container.appendChild(makeRow(shortLabel(item.at, parent), item.at, meta + (item.title && item.title !== item.at && item.title !== shortLabel(item.at, parent) ? ` <span class="cnt">${esc(item.title)}</span>` : "")));
    } else {
      // Строка данных без адреса (например, строка исходника): адреса у неё нет.
      container.appendChild(makeRow(JSON.stringify(item).slice(0, 160)));
    }
  }
  if (!(data.branches || []).length && !(data.items || []).length && !(data.sourceSets || []).length) {
    const empty = document.createElement("div");
    empty.className = "cnt";
    empty.textContent = "нет ветвей и элементов";
    container.appendChild(empty);
  }
  if (result.cursor) {
    const more = document.createElement("div");
    more.className = "row more";
    const page = result.page ? " " + JSON.stringify(result.page) : "";
    more.textContent = "ещё…" + page;
    more.onclick = async () => { more.remove(); await fill(container, at, result.cursor); };
    container.appendChild(more);
  }
}

function renderNode(at, answer) {
  const result = answer.result || {};
  const data = result.data || {};
  const node = $("node");
  const nodeAt = data.at || result.at || at || "";
  selected = nodeAt;
  $("addr").value = nodeAt;
  let html = `<h2 class="at">${esc(nodeAt || "(корень)")}</h2>`;
  html += `<dl><dt>ok</dt><dd class="${result.ok ? "yes" : "err"}">${result.ok}</dd>`;
  html += `<dt>kind</dt><dd>${esc(data.kind ?? "—")}</dd><dt>title</dt><dd>${esc(data.title ?? "—")}</dd>`;
  html += `<dt>summary</dt><dd>${esc(result.summary ?? "")}</dd><dt>время</dt><dd>${timing(answer)}</dd></dl>`;
  if (!result.ok) {
    html += `<section><h3>Отказ</h3><pre>${esc(pretty({diagnostics: result.diagnostics, next: result.next}))}</pre></section>`;
  }
  if (data.props && Object.keys(data.props).length) {
    // Длинные props свёрнуты, чтобы `can` оставался на виду.
    const count = Object.keys(data.props).length;
    html += `<section><h3>props (${count})</h3><details${count <= 12 ? " open" : ""}><summary>показать</summary><pre>${esc(pretty(data.props))}</pre></details></section>`;
  }
  if (data.limits && data.limits.length) html += `<section><h3>limits</h3><pre>${esc(pretty(data.limits))}</pre></section>`;
  if (data.branches && data.branches.length) {
    html += `<section><h3>branches</h3><table>`;
    for (const b of data.branches) html += `<tr><td><a href="#" data-at="${esc(b.at)}" class="at">${esc(b.at)}</a></td><td>${esc(b.count ?? "")}</td></tr>`;
    html += `</table></section>`;
  }
  if ("can" in data) {
    html += `<section><h3>can (${data.can.length})</h3>`;
    if (!data.can.length) {
      // Коллекция (`items`, страница) сама ничего не принимает: добавление
      // объявляет её владелец, изменение — члены.
      const owner = nodeAt.includes(".") ? nodeAt.slice(0, nodeAt.lastIndexOf(".")) : "";
      html += "items" in data
        ? `<p class="cnt">Узел-коллекция: словарь вычислен и пуст. Добавление в коллекцию объявляет владелец${owner ? ` <a href="#" data-at="${esc(owner)}" class="at">${esc(owner)}</a>` : ""}, изменение — её члены.</p>`
        : `<p class="cnt">Словарь вычислен и пуст: на этом узле ничего не применимо.</p>`;
    }
    else {
      html += `<table><tr><th>op</th><th>args</th><th>implemented</th><th></th></tr>`;
      data.can.forEach((entry, index) => {
        html += `<tr><td><code>${esc(entry.op)}</code></td><td><code>${esc(JSON.stringify(skeleton(entry.args)))}</code></td>`;
        html += `<td class="${entry.implemented ? "yes" : "no"}">${entry.implemented}</td>`;
        html += `<td><button data-contract="${index}">контракт</button> <button data-preview="${index}">предпросмотр</button></td></tr>`;
      });
      html += `</table>`;
    }
    html += `</section>`;
  } else {
    html += `<section><h3>can</h3><p class="cnt">Слота can нет: словарь для этого узла не вычисляется.</p></section>`;
  }
  html += `<section id="detail"></section>`;
  html += `<section><h3>ответ view</h3><details><summary>сырой JSON</summary><pre>${esc(pretty(answer))}</pre></details></section>`;
  node.innerHTML = html;
  node.querySelectorAll("a[data-at]").forEach((a) => a.onclick = (event) => { event.preventDefault(); select(a.dataset.at); });
  node.querySelectorAll("button[data-contract]").forEach((b) => b.onclick = () => showContract(nodeAt, data.can[+b.dataset.contract]));
  node.querySelectorAll("button[data-preview]").forEach((b) => b.onclick = () => showPreview(nodeAt, data.can[+b.dataset.preview]));
}

async function select(at, row, fresh) {
  if (selectedRow) selectedRow.classList.remove("sel");
  if (row) { row.classList.add("sel"); selectedRow = row; }
  $("node").innerHTML = `<p class="cnt" id="nodeLoading"></p>`;
  const stop = ticking($("nodeLoading"), `загрузка ${at || "(корень)"}…`);
  try { const answer = await view(at, undefined, fresh); stop(); renderNode(at, answer); }
  catch (error) { stop(); $("node").innerHTML = `<p class="err">${esc(error.message)}</p>`; }
}

async function showContract(nodeAt, entry) {
  const address = `${nodeAt}.Can.${entry.op}`;
  const detail = $("detail");
  detail.innerHTML = `<h3>Контракт ${esc(entry.op)}</h3><p class="at">${esc(address)}</p><p class="cnt">загрузка…</p>`;
  try {
    const answer = await view(address);
    const result = answer.result || {};
    const props = (result.data || {}).props;
    detail.innerHTML = `<h3>Контракт ${esc(entry.op)}</h3><p class="at">${esc(address)} <button id="openCan">открыть узел</button> ${timing(answer)}</p>` +
      (result.ok ? `<pre>${esc(pretty(props))}</pre>` : `<p class="err">отказ: ${esc(result.summary)}</p><pre>${esc(pretty({diagnostics: result.diagnostics, next: result.next}))}</pre>`);
    $("openCan").onclick = () => select(address);
  } catch (error) { detail.innerHTML += `<p class="err">${esc(error.message)}</p>`; }
}

function showPreview(nodeAt, entry) {
  const detail = $("detail");
  const draft = {at: nodeAt, ops: [{op: entry.op, args: skeleton(entry.args)}]};
  detail.innerHTML = `<h3>Предпросмотр ${esc(entry.op)}</h3>
    <p class="cnt">apply без executionToken строит план и ничего не пишет. Токен наблюдатель не исполняет.</p>
    <textarea id="draft" spellcheck="false">${esc(pretty(draft))}</textarea>
    <p><button class="primary" id="runPreview">Построить план</button></p><div id="previewOut"></div>`;
  $("runPreview").onclick = async () => {
    const out = $("previewOut");
    let body;
    try { body = JSON.parse($("draft").value); } catch (error) { out.innerHTML = `<p class="err">JSON: ${esc(error.message)}</p>`; return; }
    out.innerHTML = `<p class="cnt" id="planning"></p>`;
    const stop = ticking($("planning"), "планирование…");
    try {
      const answer = await api("/api/apply", body);
      stop();
      const result = answer.result || {};
      out.innerHTML = (result.ok ? `<p class="yes">план построен: ${esc(result.summary)} ${timing(answer)}</p>` : `<p class="err">отказ: ${esc(result.summary)} ${timing(answer)}</p>`) +
        `<pre>${esc(pretty(result))}</pre>`;
    } catch (error) { stop(); out.innerHTML = `<p class="err">${esc(error.message)}</p>`; }
  };
}

let logSeq = 0;
async function pollLog() {
  try {
    const {entries} = await api("/api/log?since=" + logSeq);
    const log = $("log");
    const stick = log.scrollTop + log.clientHeight >= log.scrollHeight - 8;
    for (const entry of entries) {
      logSeq = entry.seq;
      const p = entry.payload || {};
      const what = p.method || (p.error ? "error" : "result");
      const id = p.id !== undefined ? ` #${p.id}` : "";
      const tool = p.params && p.params.name ? ` ${p.params.name}` : "";
      const at = p.params && p.params.arguments && "at" in p.params.arguments ? ` ${p.params.arguments.at}` : "";
      const took = entry.elapsedMs != null ? ` · ${(entry.elapsedMs / 1000).toFixed(2)} с` : "";
      const progress = p.method === "notifications/progress" && p.params ? ` ${p.params.progress ?? ""}/${p.params.total ?? ""} ${p.params.message ?? ""}` : "";
      const d = document.createElement("details");
      const time = new Date(entry.time * 1000).toLocaleTimeString();
      d.innerHTML = `<summary><span class="dir-${entry.dir}">${entry.dir === "out" ? "→" : "←"}</span> ${esc(time)} ${esc(what + id + tool + at + progress + took)}</summary><pre>${esc(pretty(p))}</pre>`;
      log.appendChild(d);
    }
    if (stick) log.scrollTop = log.scrollHeight;
  } catch (error) { /* сервер остановлен */ }
  setTimeout(pollLog, 1000);
}

async function boot() {
  try {
    const info = await api("/api/info");
    $("ws").textContent = `${info.workspace} · ${(info.server || {}).name || ""} ${(info.server || {}).version || ""} · ${info.tools.join(", ")}`;
  } catch (error) { $("ws").textContent = error.message; }
  const root = makeRow("(корень)", "");
  $("tree").appendChild(root);
  root.querySelector(".tog").click();
  select("", root.querySelector(".row"));
  pollLog();
}

// Адрес, набранный руками, всегда спрашивается заново.
$("go").onclick = () => select($("addr").value.trim(), null, true);
$("addr").onkeydown = (event) => { if (event.key === "Enter") select($("addr").value.trim(), null, true); };
$("copy").onclick = async () => {
  const text = selected ?? $("addr").value;
  try { await navigator.clipboard.writeText(text); $("copy").textContent = "Скопировано"; }
  catch { $("addr").select(); document.execCommand("copy"); }
  setTimeout(() => $("copy").textContent = "Копировать адрес", 1200);
};
// Разделители панелей: ширина дерева и высота журнала, с памятью в браузере.
function store(key, value) {
  try { if (value === null) localStorage.removeItem(key); else localStorage.setItem(key, value); } catch { /* хранилище недоступно */ }
}
function restore(key) {
  try { return localStorage.getItem(key); } catch { return null; }
}
function splitter(handle, cssVar, key, measure) {
  const root = document.documentElement;
  const saved = restore(key);
  if (saved) root.style.setProperty(cssVar, saved);
  handle.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    handle.setPointerCapture(event.pointerId);
    handle.classList.add("drag");
    const move = (e) => root.style.setProperty(cssVar, measure(e) + "px");
    const up = () => {
      handle.classList.remove("drag");
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      handle.removeEventListener("pointercancel", up);
      store(key, root.style.getPropertyValue(cssVar));
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
    handle.addEventListener("pointercancel", up);
  });
  handle.addEventListener("dblclick", () => { root.style.removeProperty(cssVar); store(key, null); });
}
const clamp = (value, low, high) => Math.max(low, Math.min(high, value));
splitter($("splitTree"), "--tree-w", "unica-observer.treeWidth",
  (e) => clamp(e.clientX - document.querySelector("main").getBoundingClientRect().left, 160, window.innerWidth - 240));
splitter($("splitLog"), "--log-h", "unica-observer.logHeight",
  (e) => clamp(window.innerHeight - e.clientY - 3, 60, window.innerHeight - 200));

boot();
</script>
</body>
</html>
"""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--workspace", required=True, type=Path, help="рабочая область 1С")
    parser.add_argument("--unica", type=Path, default=None, help="бинарь unica (умолчание: target/release/unica)")
    parser.add_argument("--port", type=int, default=0, help="порт страницы на 127.0.0.1 (0 — свободный)")
    parser.add_argument("--no-open", action="store_true", help="не открывать браузер")
    parser.add_argument(
        "--timeout", type=float, default=REQUEST_TIMEOUT_SECONDS, help="предел одного вызова MCP, секунды"
    )
    args = parser.parse_args(argv)

    binary = (args.unica or default_binary()).resolve()
    if not binary.is_file():
        parser.error(f"нет бинаря unica: {binary}; соберите `cargo build --release -p unica-coder --bin unica` или укажите --unica")
    workspace = args.workspace.resolve()
    if not workspace.is_dir():
        parser.error(f"нет каталога рабочей области: {workspace}")

    try:
        client = McpClient(binary, workspace, timeout=args.timeout)
    except McpError as error:
        print(f"unica не поднялся: {error}", file=sys.stderr)
        return 1
    server = ThreadingHTTPServer(("127.0.0.1", args.port), make_handler(Observer(client), PAGE.encode()))
    url = f"http://127.0.0.1:{server.server_address[1]}/"
    print(f"UNICA_OBSERVER_URL={url}", flush=True)
    print(f"рабочая область {workspace}; Ctrl+C — остановить", file=sys.stderr, flush=True)
    if not args.no_open:
        webbrowser.open(url)

    def stop(signum: int, frame: Any) -> None:
        # SIGTERM завершает так же, как Ctrl+C: закрыть stdin unica и
        # дождаться выхода фронтенда. Общий демон уходит сам по паузе простоя.
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, stop)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        client.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
