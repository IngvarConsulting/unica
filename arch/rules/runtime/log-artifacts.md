---
id: INV.TOKEN.RUNTIME-LOG-ARTIFACTS
check:
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::terminal_snapshot_and_persistence_are_redacted_and_keep_log_artifacts
---

# Итог задания содержит пути к журналам

Захваченный вывод задания сохраняется в `stdout.log` и `stderr.log`.
Итоговый снимок содержит пути к этим файлам, а не их содержимое целиком.
Это сохранённый захваченный журнал; правило не обещает неограниченное
накопление всего вывода процесса.
