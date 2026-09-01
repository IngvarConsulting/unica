---
id: INV.SURFACE.ARGUMENTS-DESCRIBED
status: active
governs: product
decision: DEC.2026-09-01.VIEW-WORKSPACE-BOOTSTRAP
check: crates/unica-coder/src/application/v13/tool_catalog.rs::canonical_arguments_are_described_within_wire_budget
scope: [wire]
---

# Каждый опубликованный аргумент описан

Каждое свойство опубликованной схемы инструмента несёт непустое описание
достаточной длины, чтобы модель не угадывала назначение аргумента по имени.
