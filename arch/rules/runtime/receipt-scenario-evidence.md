---
id: INV.APP.RECEIPT-SCENARIO-EVIDENCE
check:
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::scenario_driver_cannot_mint_runtime_telemetry_events
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::crash_is_a_staged_control_and_does_not_mint_receipt_transition_evidence
  - tests/ci/test_receipt_harness_boundary.py::ReceiptHarnessBoundaryTests.test_live_tree_keeps_the_harness_observing
---

# Бегунок сценариев ReceiptLedger не изготавливает свидетельства

События телеметрии рантайма квитанций порождает только производственный
рантайм. Бегунок контракта передаёт ему действия и читает то, что тот
записал, но сам этих событий не порождает.

Бегунок пишет в хранилище только в ролях отдельного владельца, и каждая
такая роль с разрешёнными ей переходами названа в описи
`scripts/ci/check-receipt-harness-boundary.py`. Диспетчер действий
писателей не вызывает. Что роль не пишет за наблюдаемую попытку, держит
ревью описи, а не страж. Авария процесса как действие сценария не обрывает
сценарий и сама свидетельством перехода не служит.
