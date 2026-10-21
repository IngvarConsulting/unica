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

Переход квитанции, который наблюдает сценарий, делает рантайм, а не
бегунок. Бегунок пишет в хранилище только там, где играет отдельного
владельца: засевает состояние, создаёт нагрузку, ставит пробы, портит
индекс, ставит терминал вторым владельцем. Каждая такая роль названа в
описи `scripts/ci/check-receipt-harness-boundary.py`, а диспетчер
действий не пишет вовсе. Авария процесса как действие сценария не
обрывает сценарий и сама свидетельством перехода не служит.
