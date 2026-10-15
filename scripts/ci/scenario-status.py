#!/usr/bin/env python3
"""Собрать данные страницы сценариев из приёмочного корпуса.

Каталог сценариев на сайте не пишется руками: он порождается из того же файла,
который гоняет приёмочный тест. Значит, страница не может обещать сценарий,
которого нет в корпусе, и не отстаёт от него.
"""

from __future__ import annotations

import argparse
import collections
import importlib.util
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS_STORAGE = Path(__file__).with_name("acceptance_corpus.py")


def load_corpus(root: Path | None = None) -> dict:
    """Весь корпус тем же загрузчиком, что и приёмочный тест: закрытый блок
    `S###` и сценарии по одному в файле."""
    spec = importlib.util.spec_from_file_location("acceptance_corpus", CORPUS_STORAGE)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.load(module.ROOT if root is None else root)

# Режим сценария по его цепочке: пишет ли он файлы, только строит план или
# ничего не меняет. Каталог называется «что умеет», и без этой метки
# сценарий с превью читается как сценарий с записью.
MODE_LABEL = {"write": "запись", "plan": "план", "read": "чтение"}

# Классы ответа, которые замораживает приёмочный корпус, и их смысл словами.
# Порядок — от обычного к редкому; класс без сценариев в легенду не попадает.
CLASS_NOTES = (
    ("ok", "инструмент ответил результатом"),
    ("task", "работа ушла в фоновую задачу, вернулась квитанция"),
    ("provider", "ответ зависит от внешнего поставщика: сети или платформы"),
    ("refused", "намеренный отказ по правилу, с кодом и причиной"),
    ("unsupported", "в этой версии не поддержано, отказ типизирован"),
    ("gap", "известный пробел, причина записана в корпусе"),
    ("cancelled", "задача отменена по запросу"),
    ("failed", "задача завершилась отказом"),
)


def scenario_mode(scenario: dict) -> str:
    applies = [step for step in scenario["wire"] if step["tool"] in ("unica.apply", "unica.run")]
    if any(not (step.get("args") or {}).get("dryRun", False) for step in applies):
        return "write"
    return "plan" if applies else "read"


def build(corpus: dict) -> dict[str, object]:
    scenarios = corpus["scenarios"]
    areas = collections.Counter()
    ops = collections.Counter()
    steps = 0
    rows = []

    classes = collections.Counter()
    modes = collections.Counter()

    for scenario in scenarios:
        areas[scenario["area"]] += 1
        tools = []
        for step in scenario["wire"]:
            steps += 1
            tool = step["tool"]
            if tool not in tools:
                tools.append(tool)
            for operation in (step.get("args") or {}).get("ops") or []:
                if isinstance(operation, dict) and "op" in operation:
                    ops[operation["op"]] += 1
            for expected in step.get("expect") or []:
                classes[expected] += 1
        mode = scenario_mode(scenario)
        modes[mode] += 1
        rows.append(
            {
                "id": scenario["id"],
                "area": scenario["area"],
                "task": scenario["task"],
                "tools": " · ".join(tools),
                "mode": MODE_LABEL[mode],
                "mode_key": mode,
            }
        )

    # Порядок корпуса задан загрузчиком: закрытый блок по номерам, затем новые
    # сценарии по номеру задачи. Строковое сравнение поставило бы `S-…` раньше
    # `S001`, поэтому внутри области сохраняется порядок корпуса.
    order = {scenario["id"]: index for index, scenario in enumerate(scenarios)}
    rows.sort(key=lambda row: (row["area"], order[row["id"]]))

    # Полные цепочки уезжают на страницу отдельным блоком: таблица показывает
    # задачу, а окно деталей — вызовы с аргументами и ожидаемым классом ответа.
    detailed = [
        {
            "id": scenario["id"],
            "area": scenario["area"],
            "task": scenario["task"],
            "workspace": scenario.get("workspace", ""),
            "mode": scenario_mode(scenario),
            "profile": scenario.get("profile", "source"),
            "driver": scenario.get("driver", ""),
            "wire": [
                {
                    "tool": step["tool"],
                    "args": step.get("args") or {},
                    "expect": step.get("expect") or [],
                    # Целого ответа корпус не хранит — только то, что проверяет.
                    **{
                        key: step[key]
                        for key in ("status", "validators", "refusal", "diagnostic", "gap", "knownGap",
                                    "assertions", "fileAssertions", "captures")
                        if key in step
                    },
                }
                for step in scenario["wire"]
            ],
        }
        for scenario in scenarios
    ]
    # `</` внутри тега script закрыл бы его раньше времени; `\/` — законный JSON.
    blob = json.dumps(detailed, ensure_ascii=False, separators=(",", ":")).replace("</", "<\\/")
    return {
        "scenarios_total": f"{len(scenarios)}",
        "steps_total": f"{steps}",
        "gap_steps_total": str(sum("gap" in step.get("expect", []) for scenario in scenarios for step in scenario["wire"])),
        "gap_scenarios_total": str(sum(any("gap" in step.get("expect", []) for step in scenario["wire"]) for scenario in scenarios)),
        "areas_total": f"{len(areas)}",
        "ops_total": f"{len(ops)}",
        "areas": [
            {"area": area, "area_count": f"{count}"}
            for area, count in sorted(areas.items(), key=lambda item: (-item[1], item[0]))
        ],
        "ops": [
            {"op": op, "op_count": f"{count}"}
            for op, count in sorted(ops.items(), key=lambda item: (-item[1], item[0]))
        ],
        # Сколько сценариев действительно записывают файлы: заголовок «что умеет»
        # без этой цифры выдаёт план за запись.
        "writes_total": f"{modes['write']}",
        "plans_total": f"{modes['plan']}",
        "reads_total": f"{modes['read']}",
        "classes": [
            {
                "cls": name,
                "cls_count": f"{classes.get(name, 0)}",
                "cls_note": note,
            }
            for name, note in CLASS_NOTES
            if classes.get(name, 0)
        ],
        "scenarios": rows,
        "scenarios_json": blob,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path, help="каталог корпуса; по умолчанию tests/fixtures/acceptance")
    parser.add_argument("--out", type=Path, help="куда писать; по умолчанию stdout")
    args = parser.parse_args()

    data = build(load_corpus(args.corpus))
    rendered = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    if args.out:
        args.out.write_text(rendered, encoding="utf-8")
        print(f"written: {args.out}")
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main())
