#!/usr/bin/env python3
"""Собрать данные страницы сценариев из приёмочного корпуса.

Каталог сценариев на сайте не пишется руками: он порождается из того же файла,
который гоняет приёмочный тест. Значит, страница не может обещать сценарий,
которого нет в корпусе, и не отстаёт от него.
"""

from __future__ import annotations

import argparse
import collections
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS = REPO_ROOT / "tests" / "fixtures" / "acceptance" / "scenario-corpus.json"


def build(corpus: dict) -> dict[str, object]:
    scenarios = corpus["scenarios"]
    areas = collections.Counter()
    ops = collections.Counter()
    steps = 0
    rows = []

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
        rows.append(
            {
                "id": scenario["id"],
                "area": scenario["area"],
                "task": scenario["task"],
                "tools": " · ".join(tools),
            }
        )

    rows.sort(key=lambda row: (row["area"], row["id"]))

    # Полные цепочки уезжают на страницу отдельным блоком: таблица показывает
    # задачу, а окно деталей — вызовы с аргументами и ожидаемым классом ответа.
    detailed = [
        {
            "id": scenario["id"],
            "area": scenario["area"],
            "task": scenario["task"],
            "workspace": scenario.get("workspace", ""),
            "wire": [
                {
                    "tool": step["tool"],
                    "args": step.get("args") or {},
                    "expect": step.get("expect") or [],
                    # Целого ответа корпус не хранит — только то, что проверяет.
                    **{
                        key: step[key]
                        for key in ("status", "validators", "refusal", "diagnostic")
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
        "scenarios": rows,
        "scenarios_json": blob,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path, default=CORPUS)
    parser.add_argument("--out", type=Path, help="куда писать; по умолчанию stdout")
    args = parser.parse_args()

    data = build(json.loads(args.corpus.read_text(encoding="utf-8")))
    rendered = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    if args.out:
        args.out.write_text(rendered, encoding="utf-8")
        print(f"written: {args.out}")
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main())
