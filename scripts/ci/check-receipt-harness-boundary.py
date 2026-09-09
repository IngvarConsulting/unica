#!/usr/bin/env python3
"""Бегунок контракта ReceiptLedger наблюдает, а не продвигает.

Сценарный бегунок — обвязка контракта, а не второй рантайм. Диспетчер действий
обязан только наблюдать: durable-переходы квитанции делает production-рантайм,
к которому бегунок обращается по проводу.

Писать бегунку разрешено там, где он играет **отдельного владельца**: засев
состояния перед операцией, генератор нагрузки, воротные пробы на живом
рантайме, порча индекса, поворот поколения, удержание улик, постановка
терминала вторым владельцем поверх припаркованной попытки. Каждый такой
писатель живёт в отдельной функции, чьё имя называет эту роль, и перечислен в
описи ниже. Ни один переход не делается «за» ту попытку, которую тест
наблюдает.

Страж проверяет:

1. в теле диспетчера действий нет ни одного писателя;
2. писатели встречаются только у функций из описи «владелец → переход»;
3. бегунок не называет `ReceiptLedgerStore` и `ReceiptLedgerPort` — дверь к
   хранилищу одна, актор;
4. каждый `.rs` обвязки назван: он либо читается, либо освобождён с причиной.
   Незнакомый файл роняет стража — классифицировать его должен человек.

Владельца определяет **лексическая область**, а не отступ: методы внутри
`impl` считаются своими именами, иначе метод унаследовал бы владельца от
предыдущей функции верхнего уровня и пронёс бы запись мимо описи. Объявление
`fn имя(` вызовом не считается — иначе определение обёртки-писателя падало бы
как её вызов.
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
# Читаемые файлы обвязки. Новый файл добавляется сюда осознанно.
SCANNED_FILES = (
    "control.rs",
    "dispatch.rs",
    "scenario_hooks.rs",
    "scenario_probes.rs",
    "wire.rs",
)
# Освобождённые файлы — с причиной, а не молча.
EXEMPT_FILES = {"tests.rs": "юнит-тесты хранилища, а не обвязка"}
DISPATCHER = "run_supported_receipt_scenario_for_test"

# Durable-переходы квитанции: пишущая поверхность актора ledger (одиночная и
# пакетная) и обёртки бегунка над ней. Список закрытый: неизвестное имя не
# «разрешено по умолчанию», оно просто не считается записью, поэтому добавлять
# сюда новую команду актора обязан тот, кто её заводит. Пропущенная команда
# уже дважды стоила стражу дыры: сперва не был назван `reserve`, потом —
# девять команд разом, включая `complete_bound_task_handoff` и парный
# `complete_staged_task_handoff`, и диспетчер писал мимо обоих раз. Опись
# сверяется с `ReceiptLedgerPort`: всё, что там не чтение, стоит здесь.
WRITERS = (
    "acknowledge_direct",
    "acknowledge_direct_batch",
    "acknowledge_direct_for_scenario",
    "begin_bound_task_handoff",
    "begin_bound_task_handoff_for_scenario",
    "bind_promised_task_actor",
    "bind_reserved_actor",
    "bind_reserved_actor_batch",
    "complete_bound_task_handoff",
    "complete_staged_task_handoff",
    "expire_cancel_reserved",
    "inject_receipt_identity_collision_for_scenario",
    "mark_reserved_begun",
    "mark_reserved_begun_batch",
    "promise_task_unbound",
    "promise_task_unbound_for_scenario",
    "publish_cancelled_direct_batch",
    "publish_direct_terminal",
    "publish_direct_terminal_batch",
    "publish_direct_terminal_for_scenario",
    "publish_receipt_backed_task_terminal",
    "publish_receipt_backed_task_terminal_for_scenario",
    "reclaim_expired_tombstones",
    "request_cancel_or_reserve",
    "request_task_cancel",
    "reserve",
    "reserve_batch",
    "retain_begun_task_after_link_capacity",
    "rotate_generation_for_test",
    "stage_bound_handoff_terminal_for_scenario",
    "stage_bound_task_handoff_terminal",
    "submit_direct_batch_for_load",
)

# Опись владельцев: функция → переходы, которые ей разрешено писать. Новая
# запись здесь — это заявление «бегунок играет такого-то владельца», и она
# должна пройти ревью, а не появиться незаметно внутри диспетчера.
OWNERS: dict[str, frozenset[str]] = {
    # --- засев durable-состояния перед операцией: живой рантайм доводит
    # квитанцию до нужной фазы, потом отпускает хранилище владельцу операции.
    "seed_receipt_state": frozenset(
        {
            "acknowledge_direct",
            "begin_bound_task_handoff",
            "bind_promised_task_actor",
            "bind_reserved_actor",
            "complete_bound_task_handoff",
            "mark_reserved_begun",
            "promise_task_unbound",
            "publish_direct_terminal",
            "request_cancel_or_reserve",
            "reserve",
            "retain_begun_task_after_link_capacity",
            "stage_bound_handoff_terminal_for_scenario",
        }
    ),
    "seed_staged_cross_store_terminal": frozenset(
        {
            "begin_bound_task_handoff",
            "bind_reserved_actor",
            "mark_reserved_begun",
            "reserve",
        }
    ),
    "seed_direct_probe_terminal": frozenset({"publish_direct_terminal", "reserve"}),
    "seed_identity_collision_receipt": frozenset({"reserve"}),
    # Запись, которую ledger обязан отвергнуть: ключ не тот, и вызов есть
    # доказательство отказа.
    "attempt_mismatched_reserve": frozenset({"reserve"}),
    # --- генератор нагрузки: тысячи вызовов через пакетные входы рантайма.
    "run_direct_load": frozenset({"acknowledge_direct_batch", "submit_direct_batch_for_load"}),
    "run_lazy_cancel_storm": frozenset({"publish_cancelled_direct_batch"}),
    "submit_direct_batch_for_load": frozenset(
        {
            "bind_reserved_actor_batch",
            "mark_reserved_begun_batch",
            "publish_direct_terminal_batch",
            "reserve_batch",
        }
    ),
    # --- воротные пробы на живом рантайме: операция под воротами доводит
    # собственную попытку, а не чужую.
    "attempt_task_store_bind_under_gate_for_test": frozenset(
        {
            "begin_bound_task_handoff",
            "publish_receipt_backed_task_terminal",
            "retain_begun_task_after_link_capacity",
        }
    ),
    "bind_task_under_gate_for_test": frozenset(
        {"begin_bound_task_handoff", "complete_bound_task_handoff"}
    ),
    "cancel_under_gate_for_test": frozenset({"publish_direct_terminal"}),
    "continue_receipt_owned_attempt_for_test": frozenset(
        {"publish_receipt_backed_task_terminal"}
    ),
    "mark_reserved_begun_under_gate_for_test": frozenset({"mark_reserved_begun"}),
    # Пара «публикация терминала + ретирование квитанции», которую боевой
    # рантайм делает в `publish_staged_handoff_terminal_reply`: проба доводит
    # собственную попытку, отказ проекции отдаёт диспетчеру на наблюдение.
    "publish_staged_terminal_against_provisional_for_test": frozenset(
        {"complete_staged_task_handoff"}
    ),
    "stage_bound_handoff_terminal_for_test": frozenset({"stage_bound_task_handoff_terminal"}),
    # --- засев пулов ёмкости: тысячи строк мимо провода, но своим рантаймом.
    "seed_cancel_reserved_pool_entry_for_test": frozenset({"request_cancel_or_reserve"}),
    "seed_reserved_pool_entry_for_test": frozenset({"reserve"}),
    "seed_receipt_backed_terminal_pool_entry_for_test": frozenset(
        {"promise_task_unbound", "publish_receipt_backed_task_terminal", "reserve"}
    ),
    # --- обёртки слоя крючков: единственная дверь бегунка к командам актора.
    "acknowledge_direct_for_scenario": frozenset({"acknowledge_direct"}),
    "stage_bound_handoff_terminal_for_scenario": frozenset(
        {"stage_bound_task_handoff_terminal"}
    ),
    # --- прочие владельцы.
    "acknowledge_on_retained_actor": frozenset({"acknowledge_direct_for_scenario"}),
    "corrupt_receipt_identity_index": frozenset(
        {"inject_receipt_identity_collision_for_scenario"}
    ),
    "rotate_receipt_generation": frozenset({"rotate_generation_for_test"}),
    # --- удержание улик: сбор просроченных надгробий — не переход наблюдаемой
    # попытки. Снимок собирает мусор перед чтением, иначе каталог живых строк
    # не сходится с числом ключей.
    "reclaim_expired_receipt_evidence": frozenset({"reclaim_expired_tombstones"}),
    "snapshot_with_actor": frozenset({"reclaim_expired_tombstones"}),
    # Второй владелец поверх попытки, припаркованной между коммитом handoff и
    # созданием Task: именно это чередование и воспроизводит фикстура.
    "stage_terminal_as_second_owner": frozenset({"stage_bound_handoff_terminal_for_scenario"}),
}

FORBIDDEN_TYPES = ("ReceiptLedgerStore", "ReceiptLedgerPort")

# Дверь к хранилищу мимо актора — тоже опись, а не освобождённый файл. Эти
# фикстуры открывают store до того, как актор существует: одна поднимает
# самого актора, три сеют пулы и порчу пакетами, которых у актора нет.
STORE_OPENERS = frozenset(
    {
        "open_receipt_actor_for_scenario",
        "inject_receipt_identity_collision_for_scenario",
        "seed_receipt_backed_task_terminal_for_scenario",
        "seed_receipt_tombstones_for_scenario",
    }
)

DECLARATION = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|const|unsafe|extern)\s+)*"
    r"fn\s+([A-Za-z0-9_]+)"
)
WRITER_CALL = re.compile(r"\b(" + "|".join(sorted(WRITERS, key=len, reverse=True)) + r")\s*\(")
FORBIDDEN_TYPE = re.compile(r"\b(" + "|".join(FORBIDDEN_TYPES) + r")\b")


def offenders(path: Path, source: str) -> list[str]:
    """Владельцы записей в одном файле обвязки.

    Область функции держится стеком «имя → глубина фигурных скобок на момент
    объявления». Так метод внутри `impl` отвечает за себя, а не наследует
    владельца от предыдущей функции верхнего уровня.
    """
    found: list[str] = []
    scopes: list[tuple[str, int]] = []
    pending: tuple[str, int] | None = None
    depth = 0
    in_use = False

    for index, line in enumerate(source.split("\n"), start=1):
        declaration = DECLARATION.match(line)
        declared = declaration.group(1) if declaration else None
        enclosing = scopes[-1][0] if scopes else "<file>"
        if re.match(r"^\s*(?:pub(?:\([^)]*\))?\s+)?use\s", line):
            in_use = True

        for writer in WRITER_CALL.findall(line):
            # Объявление функции — не её вызов.
            if writer == declared:
                continue
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
        # Имя в `use` ничего не обходит: важны места, где store открывают.
        if not in_use and enclosing not in STORE_OPENERS:
            for forbidden in FORBIDDEN_TYPE.findall(line):
                found.append(f"{path.as_posix()}:{index}: `{forbidden}` bypasses the actor")

        if in_use and ";" in line:
            in_use = False
        if declared is not None:
            pending = (declared, depth)
        depth += line.count("{") - line.count("}")
        if pending is not None and depth > pending[1]:
            scopes.append(pending)
            pending = None
        while scopes and depth <= scopes[-1][1]:
            scopes.pop()

    return found


def scan(root: Path) -> list[str]:
    sources = [HARNESS_ROOT] + [HARNESS_DIR / name for name in SCANNED_FILES]
    found: list[str] = []
    for source in sources:
        path = root / source
        if not path.is_file():
            found.append(f"{source.as_posix()}: harness source is missing")
            continue
        found.extend(offenders(source, path.read_text(encoding="utf-8")))

    directory = root / HARNESS_DIR
    if directory.is_dir():
        known = set(SCANNED_FILES) | set(EXEMPT_FILES)
        # `rglob`, а не `glob`: `mod rogue;` грузит `rogue/mod.rs`, и вложенный
        # каталог иначе прошёл бы мимо и чтения, и проверки на неназванный файл.
        for path in sorted(directory.rglob("*.rs")):
            relative = path.relative_to(directory).as_posix()
            if relative not in known:
                found.append(
                    f"{(HARNESS_DIR / relative).as_posix()}: unclassified harness file; "
                    f"name it a scanned source or an exempt file in the guard"
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
