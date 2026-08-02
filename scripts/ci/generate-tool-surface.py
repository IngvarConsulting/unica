#!/usr/bin/env python3
"""Render the tool-surface ledger from the live MCP registry.

The mechanical columns -- tool name, description, published arguments and their
types -- are read from `tools/list` of a built `unica` binary, never retyped by
hand. Only the review columns (result contract today, target contract, usage
scenarios) are authored, and they live in `tool-surface-review.json`.

Run with `--check` to fail when the generated file drifts from the registry.
"""

from __future__ import annotations

import argparse
import json
import queue
import subprocess
import sys
import threading
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
REVIEW_PATH = REPO_ROOT / "spec/architecture/tool-surface-review.json"
LEDGER_PATH = REPO_ROOT / "spec/architecture/tool-surface.md"
DEFAULT_BINARY = REPO_ROOT / "target/debug/unica"

# Above this count a tool is publishing the shared XML/DSL argument list rather
# than its own, so enumerating every entry would describe the list, not the
# tool. The count itself is the review signal.
SHARED_ARGUMENT_THRESHOLD = 20

# Состояние контракта — явное поле ревью, а не догадка по тексту: считать
# метрику разбором свободной прозы значит повторить ровно ту ошибку, которую
# лечит ADR-0023.
CONTRACT_STATES = {
    "typed": "Отвечают типизированным `data`",
    "partial": "Типизированы частично: часть результата всё ещё текст",
    "job": "Отвечают снимком задания в `job`",
    "prose": "Отвечают прозой в `stdout`",
}

# Граница работы по типизации. Инструмент, который планируется снять, не
# получает нового контракта: вложение в него оплачивается дважды.
SCOPE_TITLES = {
    "in": "В границах типизации",
    "retiring": "Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`)",
    "runtime": "Вне границ: семейство runtime и build изучается отдельно",
}

GROUP_TITLES = {
    "build": "build — сборка и запуск платформы",
    "cf": "cf — корень конфигурации",
    "cfe": "cfe — расширения конфигурации",
    "code": "code — код BSL",
    "dcs": "dcs — схемы компоновки данных",
    "epf": "epf — внешние обработки",
    "erf": "erf — внешние отчёты",
    "form": "form — управляемые формы",
    "help": "help — встроенная справка",
    "interface": "interface — командный интерфейс",
    "meta": "meta — объекты метаданных",
    "mxl": "mxl — табличные макеты",
    "project": "project — рабочее пространство",
    "role": "role — роли и права",
    "runtime": "runtime — выполнение и задания",
    "source": "source — логическая адресация и ресурсы",
    "standards": "standards — стандарты 1С",
    "subsystem": "subsystem — подсистемы",
    "support": "support — поддержка поставщика",
    "template": "template — макеты объектов",
    "xdto": "xdto — пакеты XDTO",
}


def read_registry(binary: Path) -> list[dict]:
    messages = [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "unica-tool-surface", "version": "1"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    ]
    payload = "".join(json.dumps(m, ensure_ascii=False) + "\n" for m in messages)
    process = subprocess.run(
        [str(binary), "mcp"],
        input=payload,
        capture_output=True,
        text=True,
        timeout=300,
    )
    for line in process.stdout.splitlines():
        try:
            response = json.loads(line)
        except json.JSONDecodeError:
            continue
        if response.get("id") == 2:
            return response["result"]["tools"]
    raise SystemExit(f"tools/list produced no response: {process.stderr[-500:]}")


def argument_type(schema: dict) -> str:
    declared = schema.get("type")
    if isinstance(declared, str):
        return declared
    if "enum" in schema:
        return "enum"
    return "any"


def escape_cell(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ").strip()


def render_arguments(tool: dict) -> list[str]:
    schema = tool.get("inputSchema", {})
    properties = schema.get("properties", {})
    required = schema.get("required", [])
    lines: list[str] = []
    shared = len(properties) > SHARED_ARGUMENT_THRESHOLD
    shown = required if shared else sorted(properties)
    if shown:
        lines.append("| Аргумент | Тип | Обяз. | Описание |")
        lines.append("| --- | --- | --- | --- |")
        for name in shown:
            entry = properties.get(name, {})
            description = escape_cell(entry.get("description", "—"))
            lines.append(
                f"| `{name}` | {argument_type(entry)} |"
                f" {'да' if name in required else 'нет'} | {description} |"
            )
    if shared:
        if lines:
            lines.append("")
        own = "показаны выше" if shown else "не объявлено ни одного"
        lines.append(
            f"Публикует **{len(properties)}** аргументов: обязательные —"
            f" {own}, остальные приходят из общего списка"
            " `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы."
        )
    elif not shown:
        lines.append("Аргументов, кроме общих `cwd`/`dryRun`/`confirm`, нет.")
    return lines


def render(tools: list[dict], review: dict) -> str:
    published = {tool["name"] for tool in tools}
    missing = sorted(published - set(review))
    stale = sorted(set(review) - published)
    if missing:
        raise SystemExit(f"нет данных ревью для: {', '.join(missing)}")
    if stale:
        raise SystemExit(f"данные ревью для снятых инструментов: {', '.join(stale)}")

    states = {state: 0 for state in CONTRACT_STATES}
    scopes = {scope: 0 for scope in SCOPE_TITLES}
    remaining = 0
    for name in published:
        entry = review[name]
        states[entry["result"]["contract"]] += 1
        scopes[entry["scope"]] += 1
        if entry["scope"] == "in" and entry["result"]["contract"] != "typed":
            remaining += 1
    wide = sum(
        1
        for tool in tools
        if len(tool.get("inputSchema", {}).get("properties", {}))
        > SHARED_ARGUMENT_THRESHOLD
    )

    out: list[str] = []
    out.append("# Ведомость публичной поверхности инструментов")
    out.append("")
    out.append(
        "Порождается `scripts/ci/generate-tool-surface.py` из `tools/list`"
        " собранного бинаря. Руками правится только"
        " [`tool-surface-review.json`](tool-surface-review.json): контракт"
        " результата и сценарии. Имена, описания и аргументы принадлежат"
        " реестру в `crates/unica-coder/src/application/mod.rs` и"
        " `tool_contracts.rs`; здесь они лишь показаны рядом"
        " (`INV-DOC-SINGLE-RULE-OWNER`)."
    )
    out.append("")
    out.append(
        "Колонка «Результат сейчас» — наблюдение ревью, а не машинный факт:"
        " страж проверяет полноту охвата и совпадение аргументов с реестром,"
        " но не читает поведение обработчика."
    )
    out.append("")
    out.append("## Итог")
    out.append("")
    out.append(f"- Инструментов: **{len(tools)}**")
    for state, title in CONTRACT_STATES.items():
        out.append(f"- {title}: **{states[state]}**")
    out.append("")
    for scope, title in SCOPE_TITLES.items():
        out.append(f"- {title}: **{scopes[scope]}**")
    out.append(
        f"- Осталось перевести на типизированный `data` в границах работы:"
        f" **{remaining}**"
    )
    out.append(
        f"- Публикуют больше {SHARED_ARGUMENT_THRESHOLD} аргументов из общего"
        f" списка: **{wide}**"
    )
    out.append("")

    by_group: dict[str, list[dict]] = {}
    for tool in tools:
        by_group.setdefault(tool["name"].split(".")[1], []).append(tool)

    for group in sorted(by_group):
        out.append(f"## {GROUP_TITLES.get(group, group)}")
        out.append("")
        for tool in sorted(by_group[group], key=lambda item: item["name"]):
            name = tool["name"]
            entry = review[name]
            out.append(f"### `{name}`")
            out.append("")
            out.append(tool.get("description", "—"))
            out.append("")
            out.extend(render_arguments(tool))
            out.append("")
            out.append(
                f"**Результат сейчас:** {entry['result']['now']}"
                f" ({CONTRACT_STATES[entry['result']['contract']].lower()})"
            )
            out.append("")
            if entry["scope"] == "in":
                out.append(f"**Целевой контракт:** {entry['result']['target']}")
            else:
                out.append(f"**{SCOPE_TITLES[entry['scope']]}.**")
            out.append("")
            out.append("**Сценарии:**")
            out.append("")
            for scenario in entry["scenarios"]:
                out.append(f"- {scenario}")
            out.append("")
    return "\n".join(out).rstrip() + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(DEFAULT_BINARY))
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()

    binary = Path(arguments.binary)
    if not binary.is_file():
        raise SystemExit(f"нет собранного бинаря: {binary}")
    rendered = render(
        read_registry(binary),
        json.loads(REVIEW_PATH.read_text(encoding="utf-8")),
    )
    if arguments.check:
        current = LEDGER_PATH.read_text(encoding="utf-8") if LEDGER_PATH.is_file() else ""
        if current != rendered:
            print(
                "ведомость разошлась с реестром;"
                " перегенерируйте scripts/ci/generate-tool-surface.py",
                file=sys.stderr,
            )
            return 1
        print("tool surface ledger: in sync")
        return 0
    LEDGER_PATH.write_text(rendered, encoding="utf-8")
    print(f"написано: {LEDGER_PATH.relative_to(REPO_ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
