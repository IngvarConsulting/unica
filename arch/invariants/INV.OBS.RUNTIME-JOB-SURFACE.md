---
id: INV.OBS.RUNTIME-JOB-SURFACE
status: superseded
governs: product
decision: DEC.2026-09-02.V0-13-LEGACY-BATCH-1
check: crates/unica-coder/src/application/tool_contracts.rs::runtime_job_schemas_keep_execution_typed_and_controls_narrow
scope: [app, product]
---

# Долговременная runtime-работа имеет отдельную типизированную поверхность

Публичные `unica.runtime.job.*` разделяют запуск, состояние, ожидание, журналы,
список и отмену и не принимают произвольный набор аргументов выполнения.
