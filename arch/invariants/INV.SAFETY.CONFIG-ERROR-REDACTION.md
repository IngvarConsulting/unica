---
id: INV.SAFETY.CONFIG-ERROR-REDACTION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/internal_adapters.rs::production_secret_redaction_surfaces_are_closed
scope: [app, product]
---

# Ошибка конфигурации не раскрывает входные данные

Диагностика чтения или разбора общей конфигурации называет только фиксированное
имя `unica.toml` и не раскрывает абсолютный путь, исходный TOML или значения
параметров.
