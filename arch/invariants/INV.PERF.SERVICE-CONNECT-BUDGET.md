---
id: INV.PERF.SERVICE-CONNECT-BUDGET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_services.rs::service_request_kind_deadline_matrix_is_closed
scope: [app, product]
---

# Подключение к сервису ограничено коротким бюджетом

Каждый вид управляющего запроса передаёт соединителю короткий ограниченный
бюджет подключения вместо неограниченного ожидания.
