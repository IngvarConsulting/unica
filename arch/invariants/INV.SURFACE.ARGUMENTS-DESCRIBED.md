---
id: INV.SURFACE.ARGUMENTS-DESCRIBED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::every_published_argument_is_described
scope: [wire]
---

# Каждый опубликованный аргумент описан

Каждое свойство опубликованной схемы инструмента несёт непустое описание
достаточной длины, чтобы модель не угадывала назначение аргумента по имени.
