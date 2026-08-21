---
id: INV.SAFETY.STREAM-SECRET-REDACTION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/internal_adapters.rs::production_secret_redaction_surfaces_are_closed
scope: [app, product]
---

# Потоковый вывод скрывает секрет на границе фрагментов

Потоковый редактор заменяет секретное значение, даже когда имя секретного
ключа разделено между соседними фрагментами вывода.
