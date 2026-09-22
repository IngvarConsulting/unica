---
id: INV.SAFETY.RUNTIME-SECRET-REDACTION
check:
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::production_secret_key_matrix_is_redacted_from_runtime_surfaces
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::worker_handoff_never_persists_actual_argv_or_output_secrets
---

# Запись задания и его журналы скрывают распознаваемые секреты

Значения параметров `connection`, `pwd`, `password`, `token` и `secret`
заменяются на `<redacted>` в снимке задания, сохранённой записи, аргументах
команды и выдаваемых журналах. Строка подключения в аргументе `--c` также
скрывается целиком.

Передача задания отдельному worker не сохраняет исходные секретные
аргументы или распознанные секреты из его вывода в файлы задания.
Распознавание опирается на эти ключи и форму строки подключения.
