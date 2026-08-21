---
id: INV.SAFETY.CONFIG-ERROR-REDACTION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/operational_config.rs::read_errors_are_redacted_to_the_fixed_basename
scope: [app, product]
---

# Ошибка чтения конфигурации не раскрывает путь

Диагностика ошибки чтения общей конфигурации называет только фиксированное имя
`unica.toml` и не раскрывает абсолютный путь рабочего пространства.
