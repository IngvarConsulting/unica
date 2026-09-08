---
id: INV.TEST.LEDGER-HARNESS-OBSERVES
status: active
governs: process
decision: DEC.2026-09-08.LEDGER-HARNESS-OBSERVES-ONLY
check: tests/ci/test_receipt_harness_boundary.py::test_live_tree_keeps_the_harness_observing
scope: [ci, app]
---

# Бегунок контракта ledger не пишет за наблюдаемую попытку

В `receipt_scenario_v5.rs` диспетчер действий
`run_supported_receipt_scenario_for_test` не выполняет ни одного
durable-перехода квитанции: он обращается к production-демону по проводу и
читает результат. Команды ledger и обёртки бегунка над ними встречаются
только у функций из описи «владелец → переход» в
`scripts/ci/check-receipt-harness-boundary.py`, и каждая такая функция играет
владельца, отличного от наблюдаемой попытки: засев, нагрузку, порчу индекса,
поворот поколения, второго владельца поверх припаркованной попытки. Имена
`ReceiptLedgerStore` и `ReceiptLedgerPort` в бегунке запрещены — к хранилищу
он ходит только через актора.

Новая строка описи — заявление о новом владельце и проходит ревью; писатель,
появившийся вне описи или в диспетчере, роняет страж.
