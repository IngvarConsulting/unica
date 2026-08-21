---
id: INV.SAFETY.STREAM-SECRET-REDACTION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/redaction.rs::stream_redactor_covers_production_secret_keys_at_every_chunk_boundary
scope: [app, product]
---

# Потоковый вывод скрывает секрет на границе фрагментов

Потоковый редактор заменяет секретное значение, даже когда имя секретного
ключа разделено между соседними фрагментами вывода.
