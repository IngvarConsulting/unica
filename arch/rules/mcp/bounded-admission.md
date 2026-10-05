---
id: INV.WIRE.BOUNDED-ADMISSION
check:
  - crates/unica-coder/src/interfaces/mcp.rs::admission_crosses_the_former_call_quota_and_retains_exact_ownership
  - crates/unica-coder/src/interfaces/mcp.rs::admission_identifier_overflow_preserves_existing_call_ownership
  - crates/unica-coder/src/interfaces/mcp.rs::dispatcher_executes_all_calls_past_the_former_quota
---

# MCP принимает вызовы без квоты их количества

MCP frontend не отказывает в `tools/call` из-за количества выполняющихся
вызовов. Каждый принятый вызов имеет отдельного владельца и остаётся в учёте
до завершения исполнения. Отмена сама по себе не снимает вызов с учёта.

Ошибка блокировки реестра или переполнение идентификатора не допускают
повторного использования чужого владельца и не изменяют уже принятые вызовы.

Правило относится к приёму вызовов frontend. Удаление оставшихся квот
поставщиков и автоматических сроков операций разбирается в
[задаче #1119](https://github.com/IngvarConsulting/unica/issues/1119).
