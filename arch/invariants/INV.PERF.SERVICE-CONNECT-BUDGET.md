---
id: INV.PERF.SERVICE-CONNECT-BUDGET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_services.rs::cancellable_connector_uses_short_connect_budget_for_every_control_kind
scope: [app, product]
---

# Подключение к сервису ограничено коротким бюджетом

Каждый вид управляющего запроса передаёт соединителю короткий ограниченный
бюджет подключения вместо неограниченного ожидания.
