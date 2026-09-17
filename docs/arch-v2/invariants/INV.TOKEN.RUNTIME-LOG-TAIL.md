---
id: INV.TOKEN.RUNTIME-LOG-TAIL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/runtime_jobs.rs::runtime_job_lifecycle_and_log_bounds_are_complete
scope: [app, product]
---

# Выдача журналов ограничивает оба хвоста

Запрос журналов обрезает stdout и stderr по заданному числу символов, не
разрезая многобайтовые символы, и не возвращает предшествующее содержимое.
