---
id: INV.SAFETY.CONFIG-ERROR-REDACTION
check:
  - crates/unica-coder/src/infrastructure/operational_config.rs::diagnostics_never_expose_absolute_paths_raw_toml_or_values
  - crates/unica-coder/src/infrastructure/operational_config.rs::read_errors_are_redacted_to_the_fixed_basename
  - crates/unica-coder/src/infrastructure/daemon/operational_snapshot_tests.rs::operational_graph_complete_fetch_keeps_one_snapshot_then_next_call_refreshes
  - crates/unica-coder/src/infrastructure/daemon/operational_snapshot_tests.rs::operational_search_keeps_loaded_budget_and_refreshes_next_call
  - crates/unica-coder/src/infrastructure/daemon/operational_snapshot_tests.rs::operational_bsl_check_holds_loaded_timeout_and_refreshes_next_call
  - crates/unica-coder/src/infrastructure/daemon/operational_snapshot_tests.rs::operational_invalid_load_stops_all_canonical_providers_and_redacts_values
  - crates/unica-coder/src/interfaces/mcp_operational_discovery_tests.rs::mcp_construction_and_discovery_never_load_workspace_operational_settings
---

# Ответы инструментов не раскрывают значения настроек

При ошибке чтения или разбора общей конфигурации диагностика называет
`unica.toml`; если проблемное поле определено, указывает его.
В текст ошибки и её сериализованное
представление не попадают абсолютный путь, исходный TOML и значения настроек.

Эффективные операционные настройки не включаются и в предметные ответы
инструментов. Проверки охватывают ошибки загрузчика, успешные и ошибочные
канонические ответы анализа и статические схемы настоящего MCP-сервера.
