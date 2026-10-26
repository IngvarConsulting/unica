#!/usr/bin/env python3
"""Замер двери LSP на большой выгрузке (#2180).

Поднимает `unica lsp` над рабочей папкой и меряет задержку методов двери на
одном модуле, затем время вызова MCP `unica.view` в тишине и под нагрузкой
LSP: тот же демон (общий каталог состояния), второй клиент непрерывно
просит `semanticTokens` того же модуля. Выгрузку меняет только демон, но
папку лучше передать клоном (`cp -c -R` на APFS).

    python scripts/dev/measure-lsp-door.py --workspace /path/to/clone \\
        --binary target/release/unica
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
import tempfile
import threading
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT))

from tests.ci.acceptance_lsp import LspServer, document_uri, granted_space  # noqa: E402

MODULE = "main:CommonModule.ОбщегоНазначения"


def utf16_position(text: str, offset: int) -> dict:
    line = text.count("\n", 0, offset)
    start = text.rfind("\n", 0, offset) + 1
    return {"line": line, "character": len(text[start:offset].encode("utf-16-le")) // 2}


def code_occurrence(text: str, needle: str) -> int:
    """The offset right after `needle` where it is code: not in a comment or a string."""
    at = text.find(needle)
    while at >= 0:
        line = text[text.rfind("\n", 0, at) + 1:at]
        if "//" not in line and line.count('"') % 2 == 0 and not line.lstrip().startswith("|"):
            return at + len(needle)
        at = text.find(needle, at + 1)
    raise ValueError(f"{needle!r} is not in the code of the module")


def timed(call, rounds: int) -> tuple[list[float], list[object]]:
    times, answers = [], []
    for _ in range(rounds):
        started = time.perf_counter()
        answers.append(call())
        times.append((time.perf_counter() - started) * 1000)
    return times, answers


def describe(times: list[float]) -> str:
    return f"первый {times[0]:.1f} мс, медиана {statistics.median(times):.1f} мс, макс. {max(times):.1f} мс"


def outcomes(answers) -> str:
    """Every distinct outcome of the rounds: a fast refusal in some of them
    would otherwise hide in the median."""
    seen = sorted({outcome(answer) for answer in answers})
    return " | ".join(seen)


def view_failures(answers) -> int:
    return sum(not ((answer or {}).get("result") or {}).get("structuredContent", {}).get("ok")
               for answer in answers)


def outcome(answer) -> str:
    if answer is None:
        return "нет ответа"
    if "error" in answer:
        return "ошибка " + str(answer["error"].get("message"))
    result = answer.get("result")
    if result is None:
        return "null"
    if isinstance(result, list):
        return f"{len(result)} элементов"
    if isinstance(result, dict):
        if "items" in result:
            return f"{len(result['items'])} элементов"
        if "data" in result:
            return f"{len(result['data']) // 5} токенов"
        if "text" in result:
            return f"{len(result['text'])} символов"
    return "ответ"


class McpClient:
    """MCP по stdio, тот же демон: каталог состояния общий с LSP."""

    def __init__(self, binary: Path, workspace: Path, state: Path):
        from tests.ci import test_acceptance_scenarios as acceptance

        acceptance.BINARY = binary
        self.server = acceptance.AcceptanceServer(workspace, state, "2025-06-18")

    def view(self, at: str):
        return self.server.call("unica.view", {"at": at})

    def close(self) -> None:
        self.server.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=REPO_ROOT / "target/release/unica")
    parser.add_argument("--rounds", type=int, default=10)
    arguments = parser.parse_args()
    workspace = arguments.workspace.resolve()
    binary = arguments.binary.resolve()
    text = (workspace / "src/CommonModules/ОбщегоНазначения/Ext/Module.bsl").read_text(encoding="utf-8-sig")
    print(f"модуль: {text.count(chr(10)) + 1} строк, {len(text.encode())} байт")

    with tempfile.TemporaryDirectory(prefix="unica-lsp-measure-") as raw:
        state = Path(raw).resolve()
        started = time.perf_counter()
        editor = LspServer(workspace, state, binary)
        space = granted_space(editor.initialize())
        print(f"initialize: {(time.perf_counter() - started) * 1000:.0f} мс, пространство {space!r}")
        uri = document_uri(f"{MODULE}.Body", space)
        document = {"textDocument": {"uri": uri}}
        member = code_occurrence(text, "ОбщегоНазначенияКлиентСервер.")
        argument = text.find("(", member) + 1
        requests = [
            ("workspace/textDocumentContent", {"uri": uri}),
            ("textDocument/documentSymbol", document),
            ("textDocument/foldingRange", document),
            ("textDocument/semanticTokens/full", document),
            ("textDocument/signatureHelp", {**document, "position": utf16_position(text, argument)}),
            ("textDocument/completion", {**document, "position": utf16_position(text, member)}),
            ("textDocument/hover", {**document, "position": utf16_position(text, member)}),
            ("textDocument/definition", {**document, "position": utf16_position(text, member)}),
            ("textDocument/diagnostic", document),
        ]
        for method, params in requests:
            times, answers = timed(lambda: editor.request(method, params), arguments.rounds)
            print(f"{method}: {outcomes(answers)}; {describe(times)}")

        mcp = McpClient(binary, workspace, state)
        try:
            timed(lambda: mcp.view(MODULE), 3)  # прогрев
            quiet, answers = timed(lambda: mcp.view(MODULE), arguments.rounds)
            print(f"MCP unica.view без нагрузки: {describe(quiet)}; отказов {view_failures(answers)}")
            stop = threading.Event()
            load = LspServer(workspace, state, binary, name="unica-lsp-load")
            load.initialize()
            served, refused = [0], [0]

            def hammer():
                while not stop.is_set():
                    try:
                        answer = load.request("textDocument/semanticTokens/full", document)
                    except TimeoutError:
                        refused[0] += 1
                        continue
                    if answer and "result" in answer:
                        served[0] += 1
                    else:
                        refused[0] += 1

            worker = threading.Thread(target=hammer, daemon=True)
            worker.start()
            time.sleep(1)
            loaded, answers = timed(lambda: mcp.view(MODULE), arguments.rounds)
            stop.set()
            worker.join(timeout=30)
            print(f"MCP unica.view под нагрузкой LSP: {describe(loaded)}; отказов {view_failures(answers)}; "
                  f"параллельно отдано {served[0]} semanticTokens, отказов LSP {refused[0]}")
            load.close()
            after, answers = timed(lambda: mcp.view(MODULE), arguments.rounds)
            print(f"MCP unica.view снова без нагрузки: {describe(after)}; отказов {view_failures(answers)}")
        finally:
            mcp.close()
            editor.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
