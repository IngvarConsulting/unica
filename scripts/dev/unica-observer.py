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

Панель инструментов строится из `tools/list`: форма каждого инструмента
выводится из его `inputSchema`, вызов идёт через общий маршрут `/api/tool`,
который сверяет аргументы со схемой и с политикой чтения (issue #1487).
Исполнение не предлагается: API отклоняет `executionToken` в любом месте,
`run` вне предпросмотра и инструменты, которых политика не знает, — до unica,
поэтому файлы и база не меняются. Перечень — в `TOOL_POLICY` и `RUN_PREVIEW_OPS`.

Демон unica у наблюдателя свой: по умолчанию во временном каталоге
состояния, который удаляется при выходе; `--state-dir` сохраняет его между
запусками, `--shared-state` подключает общий демон пользователя.

Запуск:
  python3.12 scripts/dev/unica-observer.py --workspace <каталог>
      [--unica target/release/unica] [--port 0] [--no-open]
      [--state-dir <каталог> | --shared-state]

Первой строкой stdout печатается `UNICA_OBSERVER_URL=<адрес страницы>`.
"""
from __future__ import annotations

import argparse
import collections
import json
import os
import re
import shutil
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
STATE_VARIABLE = "UNICA_PROVIDER_STATE_DIR"
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


def task_ids(value: Any) -> set[str]:
    """Значения `taskId` на любой глубине ответа."""
    found: set[str] = set()
    if isinstance(value, dict):
        for key, item in value.items():
            if key == "taskId" and isinstance(item, str):
                found.add(item)
            else:
                found |= task_ids(item)
    elif isinstance(value, list):
        for item in value:
            found |= task_ids(item)
    return found


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
        self.own_tasks: set[str] = set()
        self.own_tasks_lock = threading.Lock()
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
                    # Задачи, выданные этому наблюдателю: только их он может
                    # опрашивать и отменять.
                    with self.own_tasks_lock:
                        self.own_tasks.update(task_ids(message.get("result")))
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

    def _call_once(self, name: str, arguments: dict[str, Any]) -> tuple[bool, dict[str, Any], str | None]:
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        structured = result.get("structuredContent")
        source_text = None
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
            # Body приходит двумя блоками: конверт JSON и исходный текст.
            # Склейка превращает успешное чтение в ошибку разбора JSON.
            if len(parts) == len(texts) == 2 and not result.get("isError"):
                try:
                    envelope = json.loads(texts[0])
                except json.JSONDecodeError:
                    envelope = None
                data = envelope.get("data") if isinstance(envelope, dict) else None
                if (isinstance(envelope, dict) and envelope.get("ok") is True
                        and isinstance(data, dict) and data.get("kind") == "Body"):
                    structured, source_text = envelope, texts[1]
            if structured is None:
                try:
                    structured = json.loads("".join(texts))
                except json.JSONDecodeError:
                    structured = {"ok": False, "summary": "".join(texts)}
        if not isinstance(structured, dict):
            raise McpError(f"{name}: structuredContent не объект: {json.dumps(structured, ensure_ascii=False)[:300]}")
        return bool(result.get("isError")), structured, source_text

    def call_tool(self, verb: str, arguments: dict[str, Any], follow_tasks: bool = True) -> dict[str, Any]:
        """Вызов инструмента со временем по стене.

        Долгий вызов сервер передаёт в задачу (`data.task`, `next` на
        `task.result`) — наблюдатель дожидается её, как агент, и отдаёт
        итоговый ответ; время включает ожидание задачи. Вызов самих
        `task.*` (`follow_tasks=False`) отдаётся как есть.
        """
        name = self.tool_name(verb)
        with self.tool_calls_lock:
            self.tool_calls += 1
            cold = self.tool_calls == 1
        started = time.monotonic()
        is_error, structured, source_text = self._call_once(name, arguments)
        handoff_ms = None
        polls = 0
        while True:
            data = structured.get("data")
            task = data.get("task") if isinstance(data, dict) else None
            # Незавершённые состояния задачи v0.13 — `queued` и `working`
            # (`task_tools.rs`); остальные терминальны.
            if not follow_tasks or not isinstance(task, dict) or task.get("status") not in ("queued", "working"):
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
            is_error, structured, source_text = self._call_once(
                self.tool_name("task.result"), {"taskId": task_id, "waitMs": 5000}
            )
        answer = {
            "isError": is_error,
            "elapsedMs": round((time.monotonic() - started) * 1000),
            "cold": cold,
            "result": structured,
        }
        if source_text is not None:
            answer["sourceText"] = source_text
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


# --- JSON Schema ------------------------------------------------------------
# Подмножество JSON Schema, которым пишутся `inputSchema` инструментов и
# `argsSchema` контрактов операций. Незнакомые ключевые слова (`format`,
# `description`, `x-unica-*`) не проверяются: окончательно аргументы проверяет
# unica, наблюдатель лишь не пропускает заведомо неверный вызов.
_JSON_TYPES = {
    "object": lambda v: isinstance(v, dict),
    "array": lambda v: isinstance(v, list),
    "string": lambda v: isinstance(v, str),
    "boolean": lambda v: isinstance(v, bool),
    "null": lambda v: v is None,
    # JSON не различает 1 и 1.0: целое число с точкой — тоже integer.
    "integer": lambda v: (isinstance(v, int) and not isinstance(v, bool)) or (isinstance(v, float) and v.is_integer()),
    "number": lambda v: isinstance(v, (int, float)) and not isinstance(v, bool),
}
# Ключевые слова, которые подмножество не проверяет и от которых зависит
# ответ: схема с ними проверяется неполно, и наблюдатель так и говорит.
UNSUPPORTED_KEYWORDS = frozenset({
    "$ref", "$dynamicRef", "patternProperties", "prefixItems", "dependentSchemas",
    "dependentRequired", "unevaluatedProperties", "unevaluatedItems", "multipleOf",
    "exclusiveMinimum", "exclusiveMaximum",
})


def schema_unsupported(schema: Any) -> list[str]:
    """Непроверяемые ключевые слова схемы на любой глубине, без повторов."""
    found: list[str] = []
    if isinstance(schema, dict):
        found += [key for key in schema if key in UNSUPPORTED_KEYWORDS]
        for key, value in schema.items():
            if key not in ("enum", "const", "default", "examples"):
                found += schema_unsupported(value)
    elif isinstance(schema, list):
        for item in schema:
            found += schema_unsupported(item)
    return sorted(set(found))


def _json_equal(left: Any, right: Any) -> bool:
    return json.dumps(left, sort_keys=True) == json.dumps(right, sort_keys=True)


def schema_errors(schema: Any, value: Any, path: str = "$") -> list[str]:
    """Нарушения схемы по пути значения; пустой список — значение подходит."""
    if schema is True or schema is None:
        return []
    if schema is False:
        return [f"{path}: not allowed"]
    if not isinstance(schema, dict):
        return []
    errors: list[str] = []
    kind = schema.get("type")
    if kind is not None:
        kinds = kind if isinstance(kind, list) else [kind]
        if not any(_JSON_TYPES.get(name, lambda v: True)(value) for name in kinds):
            return [f"{path}: expected {'/'.join(kinds)}"]
    if "const" in schema and not _json_equal(schema["const"], value):
        errors.append(f"{path}: must be {json.dumps(schema['const'], ensure_ascii=False)}")
    if "enum" in schema and not any(_json_equal(option, value) for option in schema["enum"]):
        errors.append(f"{path}: must be one of {json.dumps(schema['enum'], ensure_ascii=False)}")
    if isinstance(value, str):
        if len(value) < schema.get("minLength", 0):
            errors.append(f"{path}: shorter than {schema['minLength']}")
        if "maxLength" in schema and len(value) > schema["maxLength"]:
            errors.append(f"{path}: longer than {schema['maxLength']}")
        if "pattern" in schema:
            try:
                if re.search(schema["pattern"], value) is None:
                    errors.append(f"{path}: does not match {schema['pattern']}")
            except re.error:
                pass
    if _JSON_TYPES["number"](value):
        if "minimum" in schema and value < schema["minimum"]:
            errors.append(f"{path}: less than {schema['minimum']}")
        if "maximum" in schema and value > schema["maximum"]:
            errors.append(f"{path}: greater than {schema['maximum']}")
    if isinstance(value, list):
        if len(value) < schema.get("minItems", 0):
            errors.append(f"{path}: fewer than {schema['minItems']} items")
        if "maxItems" in schema and len(value) > schema["maxItems"]:
            errors.append(f"{path}: more than {schema['maxItems']} items")
        if schema.get("uniqueItems") and len({json.dumps(item, sort_keys=True) for item in value}) != len(value):
            errors.append(f"{path}: items are not unique")
        if "items" in schema:
            for index, item in enumerate(value):
                errors += schema_errors(schema["items"], item, f"{path}[{index}]")
        if "contains" in schema:
            matched = sum(1 for item in value if not schema_errors(schema["contains"], item))
            if matched < schema.get("minContains", 1) or matched > schema.get("maxContains", matched):
                errors.append(f"{path}: contains {matched} matching items")
    if isinstance(value, dict):
        properties = schema.get("properties") or {}
        for key in schema.get("required") or []:
            if key not in value:
                errors.append(f"{path}: missing required {key}")
        if len(value) < schema.get("minProperties", 0):
            errors.append(f"{path}: fewer than {schema['minProperties']} properties")
        if "maxProperties" in schema and len(value) > schema["maxProperties"]:
            errors.append(f"{path}: more than {schema['maxProperties']} properties")
        extra = schema.get("additionalProperties", True)
        for key, item in value.items():
            if "propertyNames" in schema:
                errors += schema_errors(schema["propertyNames"], key, f"{path}.{key}<name>")
            if key in properties:
                errors += schema_errors(properties[key], item, f"{path}.{key}")
            elif extra is False:
                errors.append(f"{path}: unexpected property {key}")
            elif isinstance(extra, dict):
                errors += schema_errors(extra, item, f"{path}.{key}")
    for branch in schema.get("allOf") or []:
        errors += schema_errors(branch, value, path)
    if "anyOf" in schema and all(schema_errors(branch, value, path) for branch in schema["anyOf"]):
        errors.append(f"{path}: matches none of anyOf")
    if "oneOf" in schema:
        matched = sum(1 for branch in schema["oneOf"] if not schema_errors(branch, value, path))
        if matched != 1:
            errors.append(f"{path}: matches {matched} of oneOf, expected exactly one")
    if "not" in schema and not schema_errors(schema["not"], value, path):
        errors.append(f"{path}: matches a forbidden schema")
    if "if" in schema:
        branch = "then" if not schema_errors(schema["if"], value, path) else "else"
        if branch in schema:
            errors += schema_errors(schema[branch], value, path)
    return list(dict.fromkeys(errors))


# --- Политика: что наблюдатель пропускает к unica --------------------------
# Наблюдатель только читает и планирует. Каждый инструмент из `tools/list`
# классифицирован здесь; инструмент, которого в перечне нет, отклоняется
# целиком (403), пока его не разберут по схеме и обработчику
# (`crates/unica-coder/src/application/v13/*.rs`).
#
#   view, check, resolve, search, diff, docs — чтение: файлов и базы не меняют.
#   task.get, task.result, task.cancel — только для taskId, которые
#       наблюдатель сам получил в ответах unica. Отмена владельца задачи не
#       проверяет (`runtime_v5.rs`, CancelTask), а демон может быть общим с
#       другими сессиями (`--shared-state`), поэтому чужой taskId — 403.
#   apply — только планирование: верхний уровень аргументов ровно `{at, ops}`
#       независимо от схемы сервера; `executionToken` (исполнение плана),
#       `dryRun`, `ifRev` и любые другие ключи — 403.
#   run — только словарь операций (вызов без `op`) или предпросмотр операции
#       из `RUN_PREVIEW_OPS` с `dryRun: true`. `dryRun: false` исполняет
#       операцию (`tool_catalog.rs`, `RunOperation`). `launch` отклоняется
#       всегда: её предпросмотр сервер строит, но наблюдателю запуск клиента
#       1С не нужен.
#
# `executionToken` запрещён везде, на любой глубине аргументов.
TOOL_POLICY: dict[str, str] = {
    "view": "read",
    "check": "read",
    "resolve": "read",
    "search": "read",
    "diff": "read",
    "docs": "read",
    "task.get": "own-task",
    "task.result": "own-task",
    "task.cancel": "own-task",
    "apply": "plan",
    "run": "preview",
}
# Операции словаря `run` с исполнением `previewApply`: при `dryRun: true`
# сервер возвращает предпросмотр без изменений. Новая операция словаря сюда
# не попадает сама: её разбирают и добавляют явно.
RUN_PREVIEW_OPS = frozenset({
    "infobase.create", "push", "pull", "make", "download", "upload",
    "infobase.dump", "infobase.restore", "extensions.list", "extensions.set",
    "apply", "reset",
})
RUN_DENIED_OPS = {"launch": "наблюдателю не нужен запуск клиента 1С"}
APPLY_PLAN_KEYS = frozenset({"at", "ops"})
# Инструменты, которые сами опрашивают задачу: их ответ отдаётся как есть,
# без ожидания задачи наблюдателем.
TASK_VERBS = frozenset({"task.get", "task.result", "task.cancel"})


def verb_of(name: str) -> str:
    return name.removeprefix("unica.")


def enforce_policy(verb: str, arguments: dict[str, Any], own_tasks: frozenset[str] | set[str] = frozenset()) -> None:
    """Отклоняет (403) всё, что исполняет, пишет или трогает чужое, до unica."""
    if contains_token(arguments):
        raise ApiError(HTTPStatus.FORBIDDEN, "observer never executes a plan: executionToken is refused")
    mode = TOOL_POLICY.get(verb)
    if mode is None:
        raise ApiError(HTTPStatus.FORBIDDEN, f"tool {verb} is not classified by the observer policy; it is not called")
    if mode == "plan":
        extra = set(arguments) - APPLY_PLAN_KEYS
        if extra:
            raise ApiError(HTTPStatus.FORBIDDEN, f"apply only plans with at and ops; refused keys {sorted(extra)}")
        return
    if mode == "own-task":
        if arguments.get("taskId") not in own_tasks:
            raise ApiError(HTTPStatus.FORBIDDEN, f"{verb}: taskId was not issued to this observer")
        return
    if mode != "preview":
        return
    op = arguments.get("op")
    if op is None:
        if "dryRun" in arguments or "args" in arguments:
            raise ApiError(HTTPStatus.FORBIDDEN, "run without op lists the dictionary; dryRun and args need an op")
        return
    if op in RUN_DENIED_OPS:
        raise ApiError(HTTPStatus.FORBIDDEN, f"run {op} is refused: {RUN_DENIED_OPS[op]}")
    if op not in RUN_PREVIEW_OPS:
        raise ApiError(HTTPStatus.FORBIDDEN, f"run {op!r} is not a known preview operation; the observer does not call it")
    if arguments.get("dryRun") is not True:
        raise ApiError(HTTPStatus.FORBIDDEN, f"run {op} executes unless dryRun is true; the observer only previews")


# --- Черновик apply по контракту операции ----------------------------------
# Скелет аргументов по ключу словаря `can` — как его строит реестр операций.
SKELETON: dict[str, Any] = {"from": "", "items": [], "values": {}, "text": "", "at": ""}
CONTRACT_UNAVAILABLE_NOTE = "контракт недоступен (#1468)"
CONTRACT_UNAVAILABLE_CODE = "unsupported_section"
_MISSING = object()


def _placeholder(schema: dict[str, Any]) -> Any:
    kind = schema.get("type")
    if isinstance(kind, list):
        kind = next((name for name in kind if name != "null"), kind[0] if kind else None)
    if kind == "string":
        return "Sample"
    if kind in ("integer", "number"):
        return schema.get("minimum", 0)
    if kind == "boolean":
        return False
    if kind == "null":
        return None
    return _MISSING


def _decoded(hint: Any, schema: dict[str, Any]) -> Any:
    # `view` отдаёт составные свойства узла (тип, значение заполнения) строкой
    # JSON; для поля-объекта контракта её разбирают.
    if isinstance(hint, str) and schema.get("type") in ("object", "array"):
        try:
            return json.loads(hint)
        except json.JSONDecodeError:
            return hint
    return hint


def sample_value(schema: Any, hint: Any = _MISSING, example: Any = _MISSING, depth: int = 0) -> Any:
    """Значение по схеме: текущее свойство узла, пример, const, default, enum.

    Кандидаты проверяются схемой; первый подходящий берётся. Объект собирается
    из обязательных полей и полей примера контракта (`exampleArgs`), массив —
    по примеру или по `minItems`.
    """
    if not isinstance(schema, dict) or depth > 12:
        return example
    if hint is not _MISSING:
        hint = _decoded(hint, schema)
    candidates: list[Any] = []
    if hint is not _MISSING:
        candidates.append(hint)
    candidates += list(schema.get("examples") or [])[:1]
    if "const" in schema:
        candidates.append(schema["const"])
    is_object = schema.get("type") == "object" or ("properties" in schema and "type" not in schema)
    if example is not _MISSING and not is_object and schema.get("type") != "array":
        candidates.append(example)
    if "default" in schema:
        candidates.append(schema["default"])
    candidates += list(schema.get("enum") or [])[:1]
    for candidate in candidates:
        if not schema_errors(schema, candidate):
            return candidate
    if is_object:
        return _sample_object(schema, hint, example, depth)
    if schema.get("type") == "array":
        item_schema = schema.get("items") if isinstance(schema.get("items"), dict) else {}
        if isinstance(example, list):
            built = [sample_value(item_schema, example=item, depth=depth + 1) for item in example]
        else:
            built = [sample_value(item_schema, depth=depth + 1) for _ in range(schema.get("minItems", 0))]
        return [item for item in built if item is not _MISSING]
    for branch_key in ("anyOf", "oneOf"):
        for branch in schema.get(branch_key) or []:
            value = sample_value(branch, hint, example, depth + 1)
            if value is not _MISSING and not schema_errors(schema, value):
                return value
    value = _placeholder(schema)
    if value is not _MISSING and schema_errors(schema, value) and "pattern" in schema:
        value = _MISSING
    return value


def _sample_object(schema: dict[str, Any], hint: Any, example: Any, depth: int) -> Any:
    properties = schema.get("properties") or {}
    hints = hint if isinstance(hint, dict) else {}
    examples = example if isinstance(example, dict) else {}
    keys = list(schema.get("required") or [])
    keys += [key for key in examples if key not in keys and (key in properties or schema.get("additionalProperties", True) is not False)]
    built: dict[str, Any] = {}
    for key in keys:
        value = sample_value(
            properties.get(key, {}),
            hints.get(key, _MISSING),
            examples.get(key, _MISSING),
            depth + 1,
        )
        if value is not _MISSING:
            built[key] = value
    if schema_errors(schema, built) and examples and not schema_errors(schema, examples):
        return examples
    return built


def build_draft(
    at: str,
    entry: dict[str, Any],
    contract: dict[str, Any] | None,
    node_props: dict[str, Any] | None,
) -> dict[str, Any]:
    """Черновик `apply` для операции `entry` из `can` узла `at`.

    Скелет `can` объединяется с образцом по `argsSchema` контракта; для
    `*.set` поля берут текущие значения свойств узла. Пустые аргументы не
    передаются: операции вроде `*.remove` их не требуют.
    """
    op = entry["op"]
    key = entry.get("args")
    skeleton = {key: SKELETON.get(key, {})} if isinstance(key, str) else {}
    args: dict[str, Any] = dict(skeleton)
    if contract is not None:
        schema = contract.get("argsSchema") or {}
        hint: Any = _MISSING
        if op.endswith(".set") and isinstance(key, str) and node_props:
            hint = {key: node_props}
        sample = sample_value(schema, hint, contract.get("exampleArgs", _MISSING))
        args = dict(sample) if isinstance(sample, dict) else {}
        properties = schema.get("properties") or {}
        for name, value in skeleton.items():
            if name not in args and not schema_errors(properties.get(name, {}), value):
                args[name] = value
    operation: dict[str, Any] = {"op": op}
    if args:
        operation["args"] = args
    return {"at": at, "ops": [operation]}


def tool_arguments(body: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    unknown = set(body) - {"tool", "arguments"}
    if unknown:
        raise ApiError(HTTPStatus.BAD_REQUEST, f"tool call accepts only tool and arguments, got {sorted(unknown)}")
    tool = body.get("tool")
    arguments = body.get("arguments", {})
    if not isinstance(tool, str) or not tool:
        raise ApiError(HTTPStatus.BAD_REQUEST, "tool must be a tool name from tools/list")
    if not isinstance(arguments, dict):
        raise ApiError(HTTPStatus.BAD_REQUEST, "arguments must be an object")
    return tool, arguments


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

    def tools(self) -> dict[str, Any]:
        """Инструменты `tools/list` со схемами и режимом политики наблюдателя."""
        return {
            "tools": [
                {
                    "name": tool.get("name"),
                    "description": tool.get("description"),
                    "inputSchema": tool.get("inputSchema") or {},
                    "policy": TOOL_POLICY.get(verb_of(tool.get("name", ""))),
                }
                for tool in self.client.tools
            ],
            "runPreviewOps": sorted(RUN_PREVIEW_OPS),
            "runDeniedOps": RUN_DENIED_OPS,
        }

    def find_tool(self, name: str) -> dict[str, Any]:
        for tool in self.client.tools:
            wire = tool.get("name", "")
            if name in (wire, verb_of(wire)):
                return tool
        raise ApiError(HTTPStatus.NOT_FOUND, f"server did not declare tool {name}")

    def call(self, body: dict[str, Any]) -> dict[str, Any]:
        """Общий вызов: политика (403), затем схема инструмента (400), затем unica."""
        name, arguments = tool_arguments(body)
        tool = self.find_tool(name)
        verb = verb_of(tool["name"])
        with self.client.own_tasks_lock:
            own_tasks = frozenset(self.client.own_tasks)
        enforce_policy(verb, arguments, own_tasks)
        errors = schema_errors(tool.get("inputSchema") or {}, arguments)
        if errors:
            raise ApiError(HTTPStatus.BAD_REQUEST, f"{tool['name']}: " + "; ".join(errors[:8]))
        answer = self.client.call_tool(verb, arguments, follow_tasks=verb not in TASK_VERBS)
        unchecked = schema_unsupported(tool.get("inputSchema"))
        return {"tool": tool["name"], "arguments": arguments, "schemaUnchecked": unchecked, **answer}

    def draft(self, body: dict[str, Any]) -> dict[str, Any]:
        """Черновик `apply` для операции `op` из `can` узла `at`.

        Читает узел и контракт `<at>.Can.<op>` и собирает аргументы по схеме
        контракта. Где контракта нет, черновик — скелет `can`, а `note`
        называет причину.
        """
        if set(body) - {"at", "op"} or not isinstance(body.get("at"), str) or not isinstance(body.get("op"), str):
            raise ApiError(HTTPStatus.BAD_REQUEST, "draft needs a string at and a string op")
        at, op = body["at"], body["op"]
        node = self.client.call_tool("view", {"at": at})["result"]
        if not node.get("ok"):
            raise ApiError(HTTPStatus.CONFLICT, f"view {at} refused: {node.get('summary')}")
        data = node.get("data") or {}
        entry = next((item for item in data.get("can") or [] if item.get("op") == op), None)
        if entry is None:
            raise ApiError(HTTPStatus.CONFLICT, f"{op} is not in the can dictionary of {at}")
        contract_at = f"{data.get('at') or at}.Can.{op}"
        answer = self.client.call_tool("view", {"at": contract_at})
        result = answer["result"]
        contract = ((result.get("data") or {}).get("props") or {}).get("contract") if result.get("ok") else None
        note = None
        if result.get("ok") and contract is None:
            status = "absent"
            note = "контракт не опубликован" + ("" if entry.get("implemented") else ": операция не реализована")
        elif result.get("ok"):
            status = "ok"
        elif CONTRACT_UNAVAILABLE_CODE in {d.get("code") for d in result.get("diagnostics") or [] if isinstance(d, dict)}:
            # Узел объявил операцию в `can`, а `view` отвечает отказом
            # `unsupported_section`: подробный контракт ещё не опубликован.
            status = "unavailable"
            note = CONTRACT_UNAVAILABLE_NOTE
        else:
            status = "refused"
            note = f"контракт не прочитан: {result.get('summary')}"
        draft = build_draft(data.get("at") or at, entry, contract, data.get("props"))
        validation: list[str] = []
        unchecked: list[str] = []
        if contract is not None:
            args = draft["ops"][0].get("args", {})
            validation = schema_errors(contract.get("argsSchema") or {}, args, "args")
            unchecked = schema_unsupported(contract.get("argsSchema"))
        return {
            "draft": draft,
            "contract": status,
            "contractAt": contract_at,
            "note": note,
            "validation": validation,
            # Ключевые слова схемы, которые наблюдатель не проверяет: пустая
            # `validation` при них не означает, что черновик подходит.
            "validationUnchecked": unchecked,
            "contractAnswer": result,
        }

    def handle(self, method: str, path: str, body: dict[str, Any]) -> dict[str, Any]:
        if method == "GET" and path == "/api/info":
            return self.info()
        if method == "GET" and path == "/api/tools":
            return self.tools()
        if method == "POST" and path == "/api/tool":
            return self.call(body)
        if method == "POST" and path == "/api/draft":
            return self.draft(body)
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
  color-scheme: light dark;
  --bg: light-dark(#ffffff, #1e1f22);
  --panel: light-dark(#f4f4f6, #2b2d30);
  --surface: light-dark(#fbfbfc, #232428);
  --text: light-dark(#1f1f23, #dfe1e5);
  --muted: light-dark(#686873, #9a9ea6);
  --faint: light-dark(#8a8a94, #80848c);
  --line: light-dark(#ececf0, #34363b);
  --line-soft: light-dark(#f0f0f3, #2c2e33);
  --line-strong: light-dark(#e3e3e8, #3c3f45);
  --control: light-dark(#d5d5db, #4a4d55);
  --control-hover: light-dark(#b8b8c0, #62666e);
  --hover: light-dark(#f0f0f3, #35373c);
  --hover-row: light-dark(#f5f6f9, #2a2c30);
  --accent: light-dark(#3478f6, #3574f0);
  --accent-strong: light-dark(#2a66d9, #4a85f5);
  --primary: light-dark(#2a66d9, #2f68d5);
  --primary-hover: light-dark(#245ac1, #295ec5);
  --accent-bg: light-dark(#e3edff, #1f3252);
  --link: light-dark(#1d4fb8, #7cacf8);
  --on-accent: #ffffff;
  --sel: light-dark(#eaf1fe, #233452);
  --ok: light-dark(#137333, #7fd18f);
  --ok-bg: light-dark(#e6f4ea, #1f3a29);
  --bad: light-dark(#b3261e, #f38b83);
  --bad-bg: light-dark(#fde7e9, #4a2426);
  --warn: light-dark(#8a5a00, #e8b84a);
  --warn-bg: light-dark(#fff4d6, #3f3217);
  --code: light-dark(#ffffff, #1f1f1f);
  --code-text: light-dark(#3b3b3b, #cccccc);
  --code-keyword: light-dark(#0000ff, #569cd6);
  --code-string: light-dark(#a31515, #ce9178);
  --code-comment: light-dark(#008000, #6a9955);
  --code-number: light-dark(#098658, #b5cea8);
  --code-property: light-dark(#0451a5, #9cdcfe);
  --code-directive: light-dark(#795e26, #dcdcaa);
  --shadow: light-dark(rgb(0 0 0 / 18%), rgb(0 0 0 / 50%));
  --mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
:root[data-theme="light"] { color-scheme: light; }
:root[data-theme="dark"] { color-scheme: dark; }
* { box-sizing: border-box; }
[hidden] { display: none !important; }
body { margin: 0; display: flex; flex-direction: column; height: 100vh; background: var(--bg); color: var(--text); font: 13px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", Arial, sans-serif; }
header { position: relative; z-index: 5; display: flex; align-items: center; flex: none; flex-wrap: wrap; gap: 6px; min-height: 40px; padding: 5px 10px 5px 12px; border-bottom: 1px solid var(--line-strong); background: var(--panel); }
header h1 { flex: none; margin: 0 10px 0 0; font-size: 12px; font-weight: 600; }
a { color: var(--link); text-underline-offset: 2px; }
button, input, select, textarea { color: inherit; font: inherit; }
button { min-height: 28px; padding: 3px 10px; border: 1px solid var(--control); border-radius: 5px; background: var(--bg); color: var(--text); font-size: 12px; cursor: pointer; }
button:hover { border-color: var(--control-hover); background: var(--hover-row); }
button.primary { border-color: var(--primary); background: var(--primary); color: var(--on-accent); }
button.primary:hover { border-color: var(--primary-hover); background: var(--primary-hover); }
button:disabled { opacity: .55; cursor: default; }
:where(button, input, select, textarea, a):focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
#addr { flex: 1 1 260px; min-width: 120px; height: 28px; padding: 4px 8px; border: 1px solid var(--control); border-radius: 5px; background: var(--bg); color: var(--text); font: 12px var(--mono); }
#appearance { display: inline-flex; align-items: center; justify-content: center; flex: none; width: 28px; padding: 0; font-size: 18px; line-height: 1; }
#appearance[aria-expanded="true"] { border-color: var(--accent); background: var(--accent-bg); }
.appearance-menu { position: absolute; top: calc(100% + 4px); right: 8px; width: 260px; max-width: calc(100vw - 16px); padding: 12px; border: 1px solid var(--control); border-radius: 8px; background: var(--bg); box-shadow: 0 10px 28px var(--shadow); }
.appearance-title { margin-bottom: 8px; color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .04em; }
.theme-options { display: flex; gap: 2px; padding: 2px; border-radius: 7px; background: var(--hover); }
.theme-options button { flex: 1 1 0; min-height: 26px; padding: 3px 4px; border: 0; border-radius: 5px; background: transparent; color: var(--muted); font-size: 11px; }
.theme-options button[aria-checked="true"] { background: var(--bg); color: var(--text); font-weight: 600; box-shadow: 0 1px 2px var(--shadow); }
main { display: grid; grid-template-columns: var(--tree-w, 300px) 6px minmax(0, 1fr); flex: 1 1 auto; min-height: 0; }
.split { position: relative; background: var(--bg); touch-action: none; user-select: none; }
.split::after { content: ""; position: absolute; background: var(--line-strong); pointer-events: none; }
.split-v { cursor: col-resize; }
.split-v::after { top: 0; bottom: 0; left: 2px; width: 1px; }
.split-h { flex: 0 0 6px; cursor: row-resize; }
.split-h::after { top: 2px; left: 0; right: 0; height: 1px; }
.split:hover::after, .split.drag::after { background: var(--accent); }
.split-v:hover::after, .split-v.drag::after { width: 2px; }
.split-h:hover::after, .split-h.drag::after { height: 2px; }
#tree, #node, #tools { min-width: 0; min-height: 0; overflow: auto; }
#tree { padding: 8px 6px; background: var(--surface); font: 12px/1.5 var(--mono); }
#node, #tools { padding: 14px 16px; }
#right { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: var(--bg); }
#right > #node, #right > #tools { flex: 1 1 auto; }
#tabs { display: flex; align-items: stretch; flex: none; height: 32px; padding: 0 6px; border-bottom: 1px solid var(--line-strong); background: var(--panel); }
#tabs button { position: relative; min-height: 32px; padding: 0 12px; border: 0; border-radius: 0; background: transparent; color: var(--muted); font-size: 12px; }
#tabs button:hover { background: var(--hover); color: var(--text); }
#tabs button.on { color: var(--text); font-weight: 600; box-shadow: inset 0 -2px 0 var(--accent); }
#tools { container-type: inline-size; }
.tool-layout { display: grid; grid-template-columns: minmax(180px, 230px) minmax(0, 1fr); gap: 20px; align-items: start; }
.toollist { display: flex; flex-direction: column; gap: 3px; }
.toollist button { display: flex; flex-wrap: wrap; align-items: baseline; gap: 5px; min-height: 30px; padding: 5px 8px; border: 1px solid transparent; background: transparent; text-align: left; }
.toollist button:hover { background: var(--hover-row); }
.toollist button.on { border-color: transparent; background: var(--sel); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
.tool-content { min-width: 0; }
@container (max-width: 560px) { .tool-layout { grid-template-columns: 1fr; } }
.row { display: flex; gap: 5px; align-items: baseline; min-height: 20px; padding: 1px 5px; border-radius: 4px; cursor: pointer; white-space: nowrap; }
.row:hover { background: var(--hover-row); }
.row.sel { background: var(--sel); }
.tog { width: 12px; flex: none; color: var(--faint); text-align: center; }
.kids { margin-left: 12px; padding-left: 4px; border-left: 1px solid var(--line-soft); }
.cnt { color: var(--muted); font-size: 11px; }
.badge { padding: 0 5px; border: 1px solid var(--line-strong); border-radius: 8px; background: var(--hover); color: var(--muted); font-size: 10px; font-weight: 400; line-height: 15px; }
.badge.read, .badge.can { border-color: transparent; background: var(--ok-bg); color: var(--ok); }
.badge.plan, .badge.preview, .badge.own-task, .badge.graph { border-color: transparent; background: var(--warn-bg); color: var(--warn); }
.badge.deny { border-color: transparent; background: var(--bad-bg); color: var(--bad); }
.form { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 8px 12px; align-items: start; margin: 12px 0; }
.form label { padding-top: 5px; color: var(--muted); font: 11px var(--mono); }
.form label b { color: var(--bad); }
.form input, .form select, .form textarea { width: 100%; padding: 5px 7px; border: 1px solid var(--control); border-radius: 5px; background: var(--bg); font: 12px var(--mono); }
.form input, .form select { min-height: 28px; }
.form textarea { min-height: 70px; }
.form .hint { grid-column: 2; margin-top: -4px; color: var(--muted); font-size: 11px; }
.note { margin: 8px 0; padding: 8px 10px; border-left: 2px solid var(--warn); border-radius: 0 4px 4px 0; background: var(--warn-bg); font-size: 12px; }
.note.bad { border-color: var(--bad); background: var(--bad-bg); }
.err { color: var(--bad); overflow-wrap: anywhere; }
.more { margin-top: 4px; color: var(--link); }
#node h2 { margin: 0 0 14px; font-size: 12px; font-weight: 600; overflow-wrap: anywhere; }
.at { font: 12px var(--mono); overflow-wrap: anywhere; }
dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 5px 14px; margin: 10px 0; font-size: 12px; }
dt { color: var(--muted); }
dd { min-width: 0; margin: 0; }
pre { max-height: 360px; overflow: auto; margin: 6px 0; padding: 10px 12px; border: 1px solid var(--line); border-radius: 6px; background: var(--code); color: var(--code-text); font: 12px/1.55 var(--mono); tab-size: 4; }
pre.source-text { max-height: min(52vh, 520px); }
.syntax-keyword { color: var(--code-keyword); }
.syntax-string { color: var(--code-string); }
.syntax-comment { color: var(--code-comment); }
.syntax-number { color: var(--code-number); }
.syntax-property { color: var(--code-property); }
.syntax-literal { color: var(--code-keyword); }
.syntax-directive { color: var(--code-directive); }
table { width: 100%; border-collapse: collapse; font-size: 12px; }
th, td { padding: 6px 8px; border-bottom: 1px solid var(--line); text-align: left; vertical-align: top; }
th { color: var(--muted); font-size: 11px; font-weight: 500; }
td code { font-size: 11px; }
.yes { color: var(--ok); }
.no { color: var(--muted); }
textarea { width: 100%; min-height: 140px; padding: 9px 10px; border: 1px solid var(--control); border-radius: 5px; background: var(--code); color: var(--code-text); font: 12px/1.55 var(--mono); tab-size: 4; }
section { margin: 16px 0; }
section h3 { margin: 0 0 8px; color: var(--muted); font-size: 11px; font-weight: 500; text-transform: uppercase; letter-spacing: .04em; }
section > details > summary { color: var(--muted); font-size: 11px; cursor: pointer; }
#log { flex: 0 0 var(--log-h, 28vh); overflow: auto; padding: 4px 10px; background: var(--surface); font: 11px/1.5 var(--mono); }
#log details { border-bottom: 1px solid var(--line-soft); }
#log summary { min-height: 22px; padding: 2px 4px; overflow: hidden; border-radius: 3px; cursor: pointer; white-space: nowrap; text-overflow: ellipsis; }
#log summary:hover { background: var(--hover-row); }
#log details[open] > summary { background: var(--hover); }
#log pre { margin: 3px 4px 8px; }
.dir-out { color: var(--link); }
.dir-in { color: var(--ok); }
@media (max-width: 760px) {
  body { height: auto; min-height: 100vh; }
  header h1 { order: 0; margin-right: auto; }
  #appearance { order: 1; }
  #addr { order: 2; flex-basis: 100%; }
  #go, #copy { order: 3; }
  main { grid-template-columns: minmax(0, 1fr); grid-template-rows: minmax(120px, 32vh) minmax(420px, 1fr); }
  .split { display: none; }
  #right { min-height: 420px; }
  #node, #tools { padding: 12px; }
  #log { flex: none; min-height: 90px; max-height: 240px; }
}
</style>
</head>
<body>
<header>
  <h1>Наблюдатель адресов</h1>
  <input id="addr" placeholder="адрес, например main:Catalog.Валюты; пусто — корень" spellcheck="false">
  <button class="primary" id="go">Открыть</button>
  <button id="copy" title="Скопировать адрес выбранного узла">Копировать адрес</button>
  <button id="appearance" type="button" title="Оформление" aria-label="Оформление" aria-haspopup="dialog" aria-controls="appearanceMenu" aria-expanded="false">◐</button>
  <div class="appearance-menu" id="appearanceMenu" role="dialog" aria-label="Оформление" hidden>
    <div class="appearance-title">Тема интерфейса</div>
    <div class="theme-options" role="radiogroup" aria-label="Тема интерфейса">
      <button type="button" role="radio" data-theme="system" aria-checked="true">Системная</button>
      <button type="button" role="radio" data-theme="light" aria-checked="false">Светлая</button>
      <button type="button" role="radio" data-theme="dark" aria-checked="false">Тёмная</button>
    </div>
  </div>
</header>
<main>
  <div id="tree"></div>
  <div class="split split-v" id="splitTree" title="Тяните, чтобы изменить ширину; двойной щелчок — по умолчанию"></div>
  <div id="right">
    <nav id="tabs"><button data-tab="node" class="on">Узел</button><button data-tab="tools">Инструменты</button></nav>
    <div id="node"><p class="cnt">Выберите узел в дереве или введите адрес.</p></div>
    <div id="tools" hidden><p class="cnt">загрузка инструментов…</p></div>
  </div>
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

function initAppearance() {
  const trigger = $("appearance");
  const menu = $("appearanceMenu");
  const radios = [...menu.querySelectorAll("[data-theme]")];
  const themes = ["system", "light", "dark"];
  const storageKey = "unica-observer.theme";
  let theme = "system";
  try {
    const saved = localStorage.getItem(storageKey);
    if (themes.includes(saved)) theme = saved;
  } catch { /* выбор живёт до перезагрузки */ }

  function choose(value, persist = true) {
    if (!themes.includes(value)) return;
    theme = value;
    document.documentElement.dataset.theme = theme;
    radios.forEach((radio) => {
      const selected = radio.dataset.theme === theme;
      radio.setAttribute("aria-checked", String(selected));
      radio.tabIndex = selected ? 0 : -1;
    });
    if (persist) {
      try { localStorage.setItem(storageKey, theme); } catch { /* хранилище недоступно */ }
    }
  }

  function close(refocus = false) {
    menu.hidden = true;
    trigger.setAttribute("aria-expanded", "false");
    if (refocus) trigger.focus();
  }

  choose(theme, false);
  close();
  trigger.addEventListener("click", () => {
    if (!menu.hidden) { close(); return; }
    menu.hidden = false;
    trigger.setAttribute("aria-expanded", "true");
    radios.find((radio) => radio.dataset.theme === theme)?.focus();
  });
  radios.forEach((radio) => radio.addEventListener("click", () => choose(radio.dataset.theme)));
  menu.addEventListener("keydown", (event) => {
    const index = radios.indexOf(event.target);
    const offsets = {ArrowLeft: -1, ArrowUp: -1, ArrowRight: 1, ArrowDown: 1};
    if (index < 0) return;
    let next;
    if (event.key === "Home") next = 0;
    else if (event.key === "End") next = radios.length - 1;
    else if (Object.hasOwn(offsets, event.key)) next = (index + offsets[event.key] + radios.length) % radios.length;
    else return;
    event.preventDefault();
    choose(radios[next].dataset.theme);
    radios[next].focus();
  });
  document.addEventListener("keydown", (event) => {
    if (!menu.hidden && event.key === "Escape") { event.preventDefault(); close(true); }
  });
  document.addEventListener("click", (event) => {
    if (!menu.hidden && !menu.contains(event.target) && !trigger.contains(event.target)) close();
  });
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

function markLeaf(row) {
  const tog = row && row.querySelector(".tog");
  if (!tog) return;
  tog.textContent = "";
  row.nextElementSibling.hidden = true;
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
    let loading = false;
    const toggle = async () => {
      if (!tog.textContent || loading) return;
      if (!loaded) {
        loading = true;
        try { await fill(kids, at); }
        catch (error) { kids.insertAdjacentHTML("beforeend", `<div class="err">ошибка страницы: ${esc(error.message)}</div>`); }
        finally { loading = false; }
        loaded = true;
      }
      if (!tog.textContent) return;
      kids.hidden = !kids.hidden;
      tog.textContent = kids.hidden ? "▸" : "▾";
    };
    tog.onclick = (event) => {
      event.stopPropagation();
      if (event.detail <= 1) toggle();
    };
    row.onclick = (event) => { if (event.detail <= 1) select(at, row); };
    row.ondblclick = (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.target !== tog) toggle();
    };
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
  if (result.cursor) {
    const more = document.createElement("div");
    more.className = "row more";
    const page = result.page ? " " + JSON.stringify(result.page) : "";
    more.textContent = "ещё…" + page;
    more.onclick = async () => { more.remove(); await fill(container, at, result.cursor); };
    container.appendChild(more);
  }
  if (!container.childElementCount) markLeaf(container.previousElementSibling);
}

function sourceTextHtml(answer) {
  return typeof answer.sourceText === "string"
    ? `<section><h3>текст</h3><pre class="source-text"><code>${esc(answer.sourceText)}</code></pre></section>` : "";
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
  html += sourceTextHtml(answer);
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
  // Открытая форма инструмента следует за выбранным узлом.
  const atField = document.querySelector('#toolFields [data-field="at"]');
  if (atField && nodeAt) { atField.value = nodeAt; atField.dispatchEvent(new Event("input", {bubbles: true})); }
}

async function select(at, row, fresh) {
  if (selectedRow) selectedRow.classList.remove("sel");
  if (row) { row.classList.add("sel"); selectedRow = row; }
  $("node").innerHTML = `<p class="cnt" id="nodeLoading"></p>`;
  const stop = ticking($("nodeLoading"), `загрузка ${at || "(корень)"}…`);
  try {
    const answer = await view(at, undefined, fresh);
    stop();
    renderNode(at, answer);
    const result = answer.result || {};
    const data = result.data || {};
    if (result.ok && !result.cursor && !["branches", "items", "sourceSets"].some((key) => (data[key] || []).length)) markLeaf(row);
  }
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

// Кнопки «подставить» для предложенных сервером продолжений (`next`):
// вызов переносится в форму своего инструмента на вкладке «Инструменты».
function nextButtons(result) {
  const next = Array.isArray(result && result.next) ? result.next : [];
  if (!next.length) return "";
  let html = `<section><h3>next (${next.length})</h3><table>`;
  next.forEach((item, index) => {
    const executes = JSON.stringify(item.args || {}).includes('"executionToken"');
    html += `<tr><td><code>${esc(item.tool || "")}</code></td><td><code>${esc(JSON.stringify(item.args || {}))}</code><div class="cnt">${esc(item.reason || "")}</div></td>`;
    html += `<td><button data-next="${index}"${executes ? ' title="исполнение плана: наблюдатель его отклонит"' : ""}>подставить</button></td></tr>`;
  });
  return html + `</table></section>`;
}

function bindNext(container, result) {
  container.querySelectorAll("button[data-next]").forEach((b) => b.onclick = () => {
    const item = result.next[+b.dataset.next];
    openTool(item.tool, item.args || {});
  });
}

// Черновик собирает сервер (`/api/draft`): скелет `can`, образец по схеме
// контракта `<узел>.Can.<op>`, текущие свойства узла для `*.set`.
async function showPreview(nodeAt, entry) {
  const detail = $("detail");
  detail.innerHTML = `<h3>Предпросмотр ${esc(entry.op)}</h3><p class="cnt" id="drafting"></p>`;
  const stop = ticking($("drafting"), "черновик по контракту…");
  let answer;
  try { answer = await api("/api/draft", {at: nodeAt, op: entry.op}); }
  catch (error) { answer = {draft: {at: nodeAt, ops: [{op: entry.op, args: skeleton(entry.args)}]}, contract: "error", note: "черновик не построен: " + error.message}; }
  stop();
  let html = `<h3>Предпросмотр ${esc(entry.op)}</h3>`;
  html += `<p class="cnt">apply без executionToken строит план и ничего не пишет. Токен наблюдатель не исполняет.</p>`;
  html += answer.note
    ? `<p class="note" id="draftNote">${esc(answer.note)} <span class="cnt">${esc(answer.contractAt || "")}</span></p>`
    : `<p class="${(answer.validationUnchecked || []).length ? "cnt" : "yes"}" id="draftNote">черновик собран по контракту <span class="cnt">${esc(answer.contractAt || "")}</span></p>`;
  if (answer.validation && answer.validation.length) html += `<p class="note bad">черновик не проходит схему контракта: ${esc(answer.validation.join("; "))}</p>`;
  if (answer.validationUnchecked && answer.validationUnchecked.length) html += `<p class="note">проверка по схеме неполная: не проверяются ${esc(answer.validationUnchecked.join(", "))}</p>`;
  const draftText = pretty(answer.draft);
  html += `<textarea id="draft" spellcheck="false" rows="${Math.min(32, draftText.split("\n").length + 1)}">${esc(draftText)}</textarea>
    <p><button class="primary" id="runPreview">Построить план</button> <button id="draftToTool">в форму apply</button></p><div id="previewOut"></div>`;
  detail.innerHTML = html;
  $("draftToTool").onclick = () => {
    try { openTool("apply", JSON.parse($("draft").value)); }
    catch (error) { $("previewOut").innerHTML = `<p class="err">JSON: ${esc(error.message)}</p>`; }
  };
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
        nextButtons(result) + `<pre>${esc(pretty(result))}</pre>`;
      bindNext(out, result);
    } catch (error) { stop(); out.innerHTML = `<p class="err">${esc(error.message)}</p>`; }
  };
}

// --- Инструменты ---------------------------------------------------------
// Форма каждого инструмента выводится из его inputSchema: обязательные поля
// помечены, перечисления — списком, умолчания подставлены, `at` — адрес
// выбранного узла. Источник истины при вызове — JSON аргументов под формой.
let toolCatalog = null;
let currentTool = null;
const POLICY_LABEL = {read: "чтение", plan: "только план", preview: "только предпросмотр", "own-task": "свои задачи"};
// Поля, которые наблюдатель не отправляет никогда: API отклонит их до unica.
const FORBIDDEN_FIELDS = new Set(["executionToken"]);

function showTab(name) {
  document.querySelectorAll("#tabs button").forEach((b) => b.classList.toggle("on", b.dataset.tab === name));
  $("node").hidden = name !== "node";
  $("tools").hidden = name !== "tools";
}

async function loadTools() {
  if (!toolCatalog) toolCatalog = await api("/api/tools");
  return toolCatalog;
}

function fieldKind(schema) {
  if (Array.isArray(schema.enum)) return "enum";
  const type = Array.isArray(schema.type) ? schema.type.find((t) => t !== "null") : schema.type;
  if (type === "boolean") return "boolean";
  if (type === "integer" || type === "number") return "number";
  if (type === "string") return "string";
  return "json";
}

// Все значения из схемы попадают в атрибуты только через esc(). Умолчание
// схемы показывается подсказкой и не отправляется: сервер подставит его сам,
// а явная отправка умолчаний ломает часть вызовов (tool_catalog.rs, #1210).
function fieldInput(name, schema, required) {
  const kind = fieldKind(schema);
  const id = `f_${esc(name)}`;
  const title = esc([schema.description, schema.pattern ? "pattern " + schema.pattern : ""].filter(Boolean).join(" · "));
  const fallback = schema.default === undefined ? "" : `умолчание: ${JSON.stringify(schema.default)}`;
  const placeholder = esc(fallback || (kind === "json" ? "JSON" : ""));
  if (kind === "enum") {
    const options = (required ? [] : [""]).concat(schema.enum);
    return `<select id="${id}" data-field="${esc(name)}" data-kind="enum" title="${title}">${options.map((v) => `<option value="${esc(v)}">${esc(v === "" ? "— " + fallback : v)}</option>`).join("")}</select>`;
  }
  if (kind === "boolean") {
    return `<select id="${id}" data-field="${esc(name)}" data-kind="boolean" title="${title}"><option value="">— ${esc(fallback)}</option><option value="true">true</option><option value="false">false</option></select>`;
  }
  if (kind === "number") {
    const bounds = (schema.minimum !== undefined ? ` min="${esc(schema.minimum)}"` : "") + (schema.maximum !== undefined ? ` max="${esc(schema.maximum)}"` : "");
    return `<input id="${id}" type="number" data-field="${esc(name)}" data-kind="number"${bounds} placeholder="${placeholder}" title="${title}">`;
  }
  if (kind === "string") return `<input id="${id}" data-field="${esc(name)}" data-kind="string" spellcheck="false" placeholder="${placeholder}" title="${title}">`;
  return `<textarea id="${id}" data-field="${esc(name)}" data-kind="json" spellcheck="false" placeholder="${placeholder}" title="${title}"></textarea>`;
}

function setField(input, value) {
  if (value === undefined) { input.value = ""; return; }
  input.value = input.dataset.kind === "json" ? pretty(value) : String(value);
}

function readFields(form) {
  const args = {};
  const errors = [];
  form.querySelectorAll("[data-field]").forEach((input) => {
    const raw = input.value;
    if (raw === "") return;
    const name = input.dataset.field;
    switch (input.dataset.kind) {
      case "boolean": args[name] = raw === "true"; break;
      case "number": args[name] = Number(raw); break;
      case "json": try { args[name] = JSON.parse(raw); } catch (error) { errors.push(`${name}: ${error.message}`); } break;
      default: args[name] = raw;
    }
  });
  return {args, errors};
}

// Начальные аргументы формы: только адрес выбранного узла в `at`;
// умолчания схемы остаются подсказками полей.
function toolPrefill(tool) {
  const props = (tool.inputSchema || {}).properties || {};
  return "at" in props && selected ? {at: selected} : {};
}

async function openTool(name, args) {
  showTab("tools");
  let catalog;
  try { catalog = await loadTools(); } catch (error) { $("tools").innerHTML = `<p class="err">${esc(error.message)}</p>`; return; }
  const wanted = String(name || "").replace(/^unica\./, "");
  const tool = catalog.tools.find((t) => t.name.replace(/^unica\./, "") === wanted);
  renderTools(catalog, tool, args);
}

function renderTools(catalog, tool, args) {
  const panel = $("tools");
  let html = `<div class="tool-layout"><div class="toollist">`;
  for (const t of catalog.tools) {
    const policy = t.policy ? `<span class="badge ${t.policy}">${esc(POLICY_LABEL[t.policy] || t.policy)}</span>` : `<span class="badge deny">запрещён</span>`;
    html += `<button data-tool="${esc(t.name)}"${tool && t.name === tool.name ? ' class="on"' : ""}>${esc(t.name)} ${policy}</button>`;
  }
  html += `</div><div class="tool-content">`;
  if (!tool) {
    panel.innerHTML = html + `<p class="cnt">Выберите инструмент. Формы строятся по inputSchema из tools/list.</p></div></div>`;
    bindToolList(panel, catalog);
    return;
  }
  currentTool = tool;
  const schema = tool.inputSchema || {};
  const props = schema.properties || {};
  const required = new Set(schema.required || []);
  html += `<h2 class="at">${esc(tool.name)}</h2><details><summary class="cnt">описание</summary><p>${esc(tool.description || "")}</p></details>`;
  if (!tool.policy) html += `<p class="note bad">Политика наблюдателя не знает этот инструмент: вызов отклоняется до unica.</p>`;
  if (tool.policy === "plan") html += `<p class="note">Только планирование {at, ops}: executionToken наблюдатель не отправляет.</p>`;
  if (tool.policy === "preview") html += `<p class="note">Без op — словарь операций. С op — только предпросмотр (dryRun: true) операций ${esc(catalog.runPreviewOps.join(", "))}; ${esc(Object.keys(catalog.runDeniedOps).join(", "))} отклоняются.</p>`;
  html += `<div class="form" id="toolFields">`;
  for (const [name, field] of Object.entries(props)) {
    if (FORBIDDEN_FIELDS.has(name)) continue;
    html += `<label for="f_${esc(name)}">${esc(name)}${required.has(name) ? " <b>*</b>" : ""}</label>${fieldInput(name, field, required.has(name))}`;
    if (field.description) html += `<div class="hint">${esc(field.description)}</div>`;
  }
  html += `</div><h3>arguments</h3><textarea id="toolArgs" spellcheck="false"></textarea>`;
  html += `<p><button class="primary" id="toolCall">Вызвать</button> <span class="err" id="toolErrors"></span></p><div id="toolOut"></div>`;
  panel.innerHTML = html + `</div></div>`;
  bindToolList(panel, catalog);
  const fields = $("toolFields");
  const initial = args === undefined ? toolPrefill(tool) : args;
  fields.querySelectorAll("[data-field]").forEach((input) => setField(input, initial[input.dataset.field]));
  $("toolArgs").value = pretty(initial);
  $("toolArgs").rows = Math.min(24, $("toolArgs").value.split("\n").length + 1);
  const sync = () => {
    const {args: built, errors} = readFields(fields);
    // Поля вне формы (например, пришедшие из `next`) сохраняются.
    let current = {};
    try { current = JSON.parse($("toolArgs").value) || {}; } catch { current = {}; }
    for (const name of Object.keys(props)) if (!FORBIDDEN_FIELDS.has(name)) delete current[name];
    $("toolArgs").value = pretty({...current, ...built});
    $("toolErrors").textContent = errors.join("; ");
  };
  fields.addEventListener("input", sync);
  fields.addEventListener("change", sync);
  $("toolCall").onclick = () => callTool(tool);
}

function bindToolList(panel, catalog) {
  panel.querySelectorAll("button[data-tool]").forEach((b) => b.onclick = () => renderTools(catalog, catalog.tools.find((t) => t.name === b.dataset.tool)));
}

async function callTool(tool) {
  const out = $("toolOut");
  let args;
  try { args = JSON.parse($("toolArgs").value || "{}"); } catch (error) { out.innerHTML = `<p class="err">JSON: ${esc(error.message)}</p>`; return; }
  out.innerHTML = `<p class="cnt" id="toolCalling"></p>`;
  const stop = ticking($("toolCalling"), `${tool.name}…`);
  try {
    const answer = await api("/api/tool", {tool: tool.name, arguments: args});
    stop();
    const result = answer.result || {};
    const unchecked = (answer.schemaUnchecked || []).length ? `<p class="note">проверка аргументов по схеме неполная: не проверяются ${esc(answer.schemaUnchecked.join(", "))}</p>` : "";
    out.innerHTML = (result.ok ? `<p class="yes">ok: ${esc(result.summary || "")} ${timing(answer)}</p>` : `<p class="err">отказ: ${esc(result.summary || "")} ${timing(answer)}</p>`) +
      unchecked + nextButtons(result) + sourceTextHtml(answer) + `<pre>${esc(pretty(result))}</pre>`;
    bindNext(out, result);
  } catch (error) { stop(); out.innerHTML = `<p class="err">${esc(error.message)}</p>`; }
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
  const root = makeRow("(корень)", "");
  $("tree").appendChild(root);
  document.querySelectorAll("#tabs button").forEach((b) => b.onclick = () => {
    if (b.dataset.tab === "tools") openTool(currentTool ? currentTool.name : undefined);
    else showTab("node");
  });
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

initAppearance();
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
    state = parser.add_mutually_exclusive_group()
    state.add_argument(
        "--state-dir", type=Path, default=None,
        help="состояние демона unica (UNICA_PROVIDER_STATE_DIR); сохраняется между запусками",
    )
    state.add_argument(
        "--shared-state", action="store_true",
        help="общий демон пользователя (~/.unica/provider-state) вместе с другими сессиями",
    )
    args = parser.parse_args(argv)

    binary = (args.unica or default_binary()).resolve()
    if not binary.is_file():
        parser.error(f"нет бинаря unica: {binary}; соберите `cargo build --release -p unica-coder --bin unica` или укажите --unica")
    workspace = args.workspace.resolve()
    if not workspace.is_dir():
        parser.error(f"нет каталога рабочей области: {workspace}")

    # Свой демон по умолчанию: с общим демоном пользователя наблюдатель видел
    # бы задачи и планы других сессий. Явный UNICA_PROVIDER_STATE_DIR в
    # окружении считается выбором вызывающего (так запускают тесты).
    env = dict(os.environ)
    scratch_state: Path | None = None
    if args.shared_state:
        env.pop(STATE_VARIABLE, None)
    elif args.state_dir is not None:
        args.state_dir.mkdir(parents=True, exist_ok=True)
        env[STATE_VARIABLE] = str(args.state_dir.resolve())  # без символических ссылок, как ниже
    elif not env.get(STATE_VARIABLE):
        # Каталог разрешается до реального пути: демон не открывает состояние
        # через символические ссылки (на macOS /var ведёт в /private/var).
        scratch_state = Path(tempfile.mkdtemp(prefix="unica-observer-state-")).resolve()
        env[STATE_VARIABLE] = str(scratch_state)
    print(f"состояние демона: {env.get(STATE_VARIABLE) or 'общее пользователя'}", file=sys.stderr, flush=True)

    try:
        client = McpClient(binary, workspace, env=env, timeout=args.timeout)
    except McpError as error:
        print(f"unica не поднялся: {error}", file=sys.stderr)
        if scratch_state is not None:
            shutil.rmtree(scratch_state, ignore_errors=True)
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
        if scratch_state is not None:
            # Временное состояние принадлежит только этому наблюдателю.
            shutil.rmtree(scratch_state, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
