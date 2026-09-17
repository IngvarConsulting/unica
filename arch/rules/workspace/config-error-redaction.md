---
id: INV.SAFETY.CONFIG-ERROR-REDACTION
check:
  - crates/unica-coder/src/infrastructure/operational_config.rs::diagnostics_never_expose_absolute_paths_raw_toml_or_values
  - crates/unica-coder/src/infrastructure/operational_config.rs::read_errors_are_redacted_to_the_fixed_basename
---

# Ошибка чтения настроек не раскрывает их значения

При ошибке чтения или разбора общей конфигурации диагностика называет
`unica.toml`; если проблемное поле определено, указывает его.
В текст ошибки и её сериализованное
представление не попадают абсолютный путь, исходный TOML и значения настроек.
