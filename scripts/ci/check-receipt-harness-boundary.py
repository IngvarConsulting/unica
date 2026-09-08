#!/usr/bin/env python3
"""Бегунок контракта ReceiptLedger наблюдает, а не продвигает.

Сценарный бегунок (`receipt_scenario_v5.rs`) — обвязка контракта, а не второй
рантайм. Диспетчер действий обязан только наблюдать: durable-переходы
квитанции делает production-рантайм, к которому бегунок обращается по проводу.

Писать бегунку разрешено там, где он играет **отдельного владельца**: засев
состояния перед операцией, генератор нагрузки, порча индекса, поворот
поколения, постановка терминала вторым владельцем поверх припаркованной
попытки. Каждый такой писатель живёт в отдельной функции, чьё имя называет
эту роль, и перечислен в описи ниже. Ни один переход не делается «за» ту
попытку, которую тест наблюдает.

Страж проверяет три вещи:

1. в теле диспетчера действий нет ни одного писателя;
2. писатели встречаются только у функций из описи;
3. бегунок не называет `ReceiptLedgerStore` и `ReceiptLedgerPort` — дверь к
   хранилищу одна, актор.

Обвязка разложена по файлам, и каждый `.rs` рядом с ней обязан быть назван:
либо это файл-водитель (проверяется), либо явно освобождённый слой с причиной.
Незнакомый файл роняет стража — классифицировать его должен человек.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

HARNESS_ROOT = Path(
    "crates/unica-coder/src/infrastructure/daemon/runtime_v5/receipt_scenario_v5.rs"
)
HARNESS_DIR = Path(
    "crates/unica-coder/src/infrastructure/daemon/runtime_v5/receipt_scenario_v5"
)
# Файлы-водители: их и проверяем. Новый водитель добавляется сюда осознанно.
DRIVER_FILES = ("control.rs", "dispatch.rs", "wire.rs")
# Освобождённые слои — с причиной, а не молча.
EXEMPT_FILES = {
    "scenario_hooks.rs": "слой крючков: здесь и живут обёртки-писатели и дверь к актору",
    "scenario_probes.rs": "пробы рантайма, а не действия сценария",
    "tests.rs": "юнит-тесты хранилища, а не обвязка",
}
DISPATCHER = "run_supported_receipt_scenario_for_test"

# Durable-переходы квитанции: команды актора и обёртки бегунка над ними.
WRITERS = (
    "acknowledge_direct",
    "acknowledge_direct_batch",
    "acknowledge_direct_for_scenario",
    "begin_bound_task_handoff",
    "begin_bound_task_handoff_for_scenario",
    "inject_receipt_identity_collision_for_scenario",
    "promise_task_unbound",
    "promise_task_unbound_for_scenario",
    "publish_direct_terminal",
    "publish_direct_terminal_for_scenario",
    "publish_receipt_backed_task_terminal",
    "publish_receipt_backed_task_terminal_for_scenario",
    "request_cancel_or_reserve",
    "rotate_generation_for_test",
    "stage_bound_handoff_terminal_for_scenario",
    "stage_bound_task_handoff_terminal",
    "submit_direct_batch_for_load",
)

# Опись владельцев: функция → переходы, которые ей разрешено писать. Новая
# запись здесь — это заявление «бегунок играет такого-то владельца», и она
# должна пройти ревью, а не появиться незаметно внутри диспетчера.
OWNERS: dict[str, frozenset[str]] = {
    # Засев durable-состояния перед операцией: живой рантайм доводит квитанцию
    # до нужной фазы, потом отпускает хранилище владельцу операции.
    "seed_receipt_state": frozenset(
        {
            "acknowledge_direct",
            "begin_bound_task_handoff",
            "promise_task_unbound",
            "publish_direct_terminal",
            "request_cancel_or_reserve",
            "stage_bound_handoff_terminal_for_scenario",
        }
    ),
    "seed_staged_cross_store_terminal": frozenset({"begin_bound_task_handoff"}),
    "seed_direct_probe_terminal": frozenset({"publish_direct_terminal"}),
    # Генератор нагрузки: тысячи вызовов через пакетные входы живого рантайма.
    "run_direct_load": frozenset(
        {"acknowledge_direct_batch", "submit_direct_batch_for_load"}
    ),
    # Ретенированный актор без демона: слушателя нет, владелец один.
    "acknowledge_on_retained_actor": frozenset({"acknowledge_direct_for_scenario"}),
    # Порча индекса — то, чего не пишет ни один здоровый владелец.
    "corrupt_receipt_identity_index": frozenset(
        {"inject_receipt_identity_collision_for_scenario"}
    ),
    "rotate_receipt_generation": frozenset({"rotate_generation_for_test"}),
    # Второй владелец поверх попытки, припаркованной между коммитом handoff и
    # созданием Task: именно это чередование и воспроизводит фикстура.
    "stage_terminal_as_second_owner": frozenset(
        {"stage_bound_handoff_terminal_for_scenario"}
    ),
}

FORBIDDEN_TYPES = ("ReceiptLedgerStore", "ReceiptLedgerPort")

TOP_LEVEL_FN = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)"
)
WRITER_CALL = re.compile(
    r"\b(" + "|".join(sorted(WRITERS, key=len, reverse=True)) + r")\s*\("
)
FORBIDDEN_TYPE = re.compile(r"\b(" + "|".join(FORBIDDEN_TYPES) + r")\b")


def offenders(path: Path, source: str) -> list[str]:
    found: list[str] = []
    enclosing = "<file>"
    for index, line in enumerate(source.split("\n"), start=1):
        match = TOP_LEVEL_FN.match(line)
        if match:
            enclosing = match.group(1)
        for writer in WRITER_CALL.findall(line):
            if enclosing == DISPATCHER:
                found.append(
                    f"{path.as_posix()}:{index}: the action dispatcher writes "
                    f"`{writer}`; give the owner a named helper"
                )
            elif writer not in OWNERS.get(enclosing, frozenset()):
                found.append(
                    f"{path.as_posix()}:{index}: `{enclosing}` writes `{writer}`, "
                    f"which the owner inventory does not allow"
                )
        for forbidden in FORBIDDEN_TYPE.findall(line):
            found.append(
                f"{path.as_posix()}:{index}: `{forbidden}` bypasses the actor"
            )
    return found


def scan(root: Path) -> list[str]:
    drivers = [HARNESS_ROOT] + [HARNESS_DIR / name for name in DRIVER_FILES]
    found: list[str] = []
    for driver in drivers:
        path = root / driver
        if not path.is_file():
            found.append(f"{driver.as_posix()}: harness source is missing")
            continue
        found.extend(offenders(driver, path.read_text(encoding="utf-8")))

    # Каждый файл обвязки обязан быть назван: водитель или освобождённый слой.
    directory = root / HARNESS_DIR
    if directory.is_dir():
        known = set(DRIVER_FILES) | set(EXEMPT_FILES)
        for path in sorted(directory.glob("*.rs")):
            if path.name not in known:
                found.append(
                    f"{(HARNESS_DIR / path.name).as_posix()}: unclassified harness file; "
                    f"name it a driver or an exempt layer in the guard"
                )
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    arguments = parser.parse_args()
    found = scan(arguments.root)
    for line in found:
        print(line)
    if found:
        print(f"{len(found)} harness writes outside the owner inventory", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
