---
id: INV.OBS.RUNTIME-JOB-SURFACE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::runtime_job_schemas_keep_execution_typed_and_controls_narrow
scope: [app, product]
---

# Долговременная runtime-работа имеет отдельную типизированную поверхность

Публичные `unica.runtime.job.*` разделяют запуск, состояние, ожидание, журналы,
список и отмену и не принимают произвольный набор аргументов выполнения.
