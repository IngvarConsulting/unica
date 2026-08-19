---
id: INV.WIRE.DATA-DRIVEN-SCHEMA
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::every_published_argument_is_described
scope: [wire]
---

# Контракты инструментов заданы данными

Схема вызова выводится из реестра описателей, а не из макросов SDK: контракт можно прочитать
и проверить, не запуская сервер.
