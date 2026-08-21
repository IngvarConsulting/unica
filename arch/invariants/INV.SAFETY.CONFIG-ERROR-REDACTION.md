---
id: INV.SAFETY.CONFIG-ERROR-REDACTION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/operational_config.rs::diagnostics_never_expose_absolute_paths_raw_toml_or_values
scope: [app, product]
---

# Ошибка конфигурации не раскрывает входные данные

Диагностика чтения или разбора общей конфигурации называет только фиксированное
имя `unica.toml` и не раскрывает абсолютный путь, исходный TOML или значения
параметров.
