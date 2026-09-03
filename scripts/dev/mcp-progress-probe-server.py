#!/usr/bin/env python3
"""Зонд доставки прогресса и отмены на живом хосте.

Доставка движка идёт минутами, и всё, на чём держится решение «вызов ждёт», —
это готовность хоста терпеть долгий вызов, пока идут уведомления о прогрессе.
Факт не измерен ни для одного хоста. Сервер отвечает на один вопрос: что хост
делает с длинным вызовом — дожидается, обрывает, шлёт ли отмену, и просит ли
прогресс вообще.

Env:
  PROBE_LOG=<path.jsonl>   журнал каждого кадра JSON-RPC (in/out) со временем

Инструмент:
  probe_long_call {seconds, progressEveryMs}
    держит вызов `seconds` секунд, посылая прогресс каждые `progressEveryMs`
    миллисекунд (0 — молча), и отвечает терминальным результатом.
"""
from __future__ import annotations

import json
import os
import sys
import threading
import time

LOG_PATH = os.environ.get("PROBE_LOG")
# Маркер прогона: один и тот же нонс уходит и в прогресс, и в результат, чтобы
# положительный контроль и измеряемое место оказались в одной стенограмме.
MARKER = os.environ.get("PROBE_MARKER", "NONCE")
STARTED = time.monotonic()
log_lock = threading.Lock()
out_lock = threading.Lock()
cancelled = set()


def log(direction: str, message, **extra) -> None:
    if not LOG_PATH:
        return
    record = {
        "at": round(time.monotonic() - STARTED, 3),
        "direction": direction,
        "message": message,
    }
    record.update(extra)
    with log_lock:
        with open(LOG_PATH, "a") as stream:
            stream.write(json.dumps(record, ensure_ascii=False) + "\n")


def send(obj) -> None:
    log("out", obj)
    with out_lock:
        sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
        sys.stdout.flush()


def long_call(msg_id, token, seconds: float, every_ms: int) -> None:
    """Держит вызов, пока хост его терпит."""
    deadline = time.monotonic() + seconds
    sent = 0
    while time.monotonic() < deadline:
        if msg_id in cancelled:
            log("note", {"event": "abandoned-after-cancel", "id": msg_id, "progress": sent})
            return
        if every_ms:
            time.sleep(min(every_ms / 1000.0, max(0.0, deadline - time.monotonic())))
            if token is not None and time.monotonic() < deadline:
                sent += 1
                send(
                    {
                        "jsonrpc": "2.0",
                        "method": "notifications/progress",
                        "params": {
                            "progressToken": token,
                            "progress": sent,
                            "total": int(seconds * 1000 / every_ms),
                            "message": f"PROGRESS-{MARKER} tick={sent} receivedBytes={sent * 1024}",
                        },
                    }
                )
        else:
            time.sleep(min(0.25, max(0.0, deadline - time.monotonic())))
    send(
        {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": f"RESULT-{MARKER} seconds={seconds} progressSent={sent}",
                    }
                ]
            },
        }
    )


def handle(msg):
    method = msg.get("method")
    msg_id = msg.get("id")
    params = msg.get("params") or {}

    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "protocolVersion": params.get("protocolVersion") or "2025-11-25",
                "capabilities": {"tools": {}, "prompts": {"listChanged": False}},
                "serverInfo": {"name": "progress-probe", "version": "1.0.0"},
            },
        }
    if method == "ping":
        return {"jsonrpc": "2.0", "id": msg_id, "result": {}}
    if method == "notifications/initialized":
        return None
    if method == "notifications/cancelled":
        cancelled.add(params.get("requestId"))
        log("note", {"event": "cancelled-received", "params": params})
        return None
    if method == "prompts/list":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "prompts": [
                    {
                        "name": "probe_prompt",
                        "description": "PROMPT-LIST-MARKER-4B7E probe prompt for context-visibility measurement",
                        "arguments": [],
                    }
                ]
            },
        }
    if method == "prompts/get":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "description": "PROMPT-LIST-MARKER-4B7E",
                "messages": [
                    {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": "PROMPT-BODY-MARKER-8C2D the probe prompt body",
                        },
                    }
                ],
            },
        }
    if method == "tools/list":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "tools": [
                    {
                        "name": "probe_long_call",
                        "description": (
                            "Probe: holds the call for the given number of seconds while "
                            "emitting progress. Call it exactly as the user asks."
                        ),
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "seconds": {"type": "number"},
                                "progressEveryMs": {"type": "integer"},
                            },
                            "required": ["seconds"],
                        },
                    }
                ]
            },
        }
    if method == "tools/call":
        arguments = params.get("arguments") or {}
        token = (params.get("_meta") or {}).get("progressToken")
        log(
            "note",
            {
                "event": "call-received",
                "id": msg_id,
                "progressToken": token,
                "hasProgressToken": token is not None,
                "arguments": arguments,
            },
        )
        try:
            seconds = float(arguments.get("seconds", 5))
            every_ms = int(arguments.get("progressEveryMs", 0))
        except (TypeError, ValueError) as error:
            return {
                "jsonrpc": "2.0",
                "id": msg_id,
                "error": {
                    "code": -32602,
                    "message": f"invalid probe arguments seconds/progressEveryMs: {error}",
                },
            }
        threading.Thread(
            target=long_call,
            args=(
                msg_id,
                token,
                seconds,
                every_ms,
            ),
            daemon=True,
        ).start()
        return None
    if msg_id is not None:
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "error": {"code": -32601, "message": f"method not found: {method}"},
        }
    return None


def main() -> int:
    log(
        "note",
        {
            "event": "start",
            "pid": os.getpid(),
            "env": {
                name: value
                for name, value in os.environ.items()
                if "TIMEOUT" in name.upper() or name.upper().startswith(("MCP", "CLAUDE"))
            },
        },
    )
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            log("note", {"unparsed": line[:200]})
            continue
        log("in", msg)
        response = handle(msg)
        if response is not None:
            send(response)
    log("note", {"event": "eof"})
    return 0


if __name__ == "__main__":
    sys.exit(main())
