---
id: INV.SAFETY.STREAM-SECRET-REDACTION
check:
  - crates/unica-coder/src/infrastructure/redaction.rs::stream_redactor_covers_production_secret_keys_at_every_chunk_boundary
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::production_secret_key_matrix_is_redacted_from_runtime_surfaces
---

# Разрыв вывода на фрагменты не раскрывает распознаваемый секрет

Потоковый редактор заменяет значение распознаваемого секретного ключа
на `<redacted>`, даже если граница соседних фрагментов проходит внутри
имени ключа или его значения. Это относится к ключам `connection`, `pwd`,
`password`, `token` и `secret`.
