---
id: INV.SURFACE.PUBLISHED-ARGS-ARE-READ
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::no_published_argument_is_described_as_unread
scope: [wire]
---

# Снятые непрочитанные аргументы не возвращаются в схему
Закрытый список аргументов, снятых как непрочитанные, отсутствует в публичных
схемах; таблица описаний также не содержит аргумент с пометкой, что обработчик
его не читает.
