---
id: INV.APP.RECEIPT-SCENARIO-EVIDENCE
check:
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::scenario_driver_cannot_mint_runtime_telemetry_events
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::crash_is_a_staged_control_and_does_not_mint_receipt_transition_evidence
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::every_malformed_envelope_case_is_routed_individually_through_the_production_pipeline
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::scenario_wire_is_closed_at_the_scenario_and_action_levels
---

# Бегунок сценариев ReceiptLedger не изготавливает свидетельства

События телеметрии рантайма квитанций и свидетельства их переходов
порождает только производственный рантайм. Бегунок контракта передаёт
ему действия и читает то, что тот записал. Сам он событий не чеканит
и наблюдений по имени сценария не синтезирует.

Провод сценария закрыт: поле, которого нет в схеме сценария или действия,
отвергается целиком. Так в рантайм не передаётся ожидаемый исход.
Некорректный внешний конверт бегунок отправляет как есть, и отвергает его
производственный разбор запроса. Управляющее действие сценария, например
авария процесса, ставит условие для следующей операции и само переходом
не считается.
