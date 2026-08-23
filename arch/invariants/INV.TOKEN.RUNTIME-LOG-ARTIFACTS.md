---
id: INV.TOKEN.RUNTIME-LOG-ARTIFACTS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/runtime_jobs.rs::terminal_snapshot_and_persistence_are_redacted_and_keep_log_artifacts
scope: [app, product]
---

# Снимок задания возвращает пути к файловым журналам

Терминальное состояние задания сохраняет `stdout.log` и `stderr.log` как
артефакты и возвращает пути к ним вместо содержимого файлов целиком.
