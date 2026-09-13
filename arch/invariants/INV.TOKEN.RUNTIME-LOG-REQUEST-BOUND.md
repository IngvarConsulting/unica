---
id: INV.TOKEN.RUNTIME-LOG-REQUEST-BOUND
status: superseded
governs: product
decision: DEC.2026-09-02.V0-13-LEGACY-BATCH-1
check: crates/unica-coder/src/application/tool_contracts.rs::runtime_job_controls_reject_invalid_ids_bounds_and_execution_arguments
scope: [app, product]
---

# Публичный запрос хвоста журнала ограничен

`unica.runtime.job.logs` принимает только положительный `tailChars` не больше
32768 и отвергает значение вне границы до выполнения операции.
