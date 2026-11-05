---
id: INV.CONFIG.OPERATIONAL-CALL-SNAPSHOT
check:
  - crates/unica-coder/src/infrastructure/daemon/operational_snapshot_tests.rs::operational_graph_complete_fetch_keeps_one_snapshot_then_next_call_refreshes
  - crates/unica-coder/src/infrastructure/daemon/operational_snapshot_tests.rs::operational_search_keeps_loaded_budget_and_refreshes_next_call
  - crates/unica-coder/src/infrastructure/daemon/operational_snapshot_tests.rs::operational_bsl_check_holds_loaded_timeout_and_refreshes_next_call
  - crates/unica-coder/src/infrastructure/daemon/operational_snapshot_tests.rs::operational_invalid_load_stops_all_canonical_providers_and_redacts_values
  - crates/unica-coder/src/infrastructure/daemon/operational_snapshot_tests.rs::operational_cancellation_during_failed_load_wins_at_actor_publication
  - crates/unica-coder/src/interfaces/mcp_operational_discovery_tests.rs::mcp_construction_and_discovery_never_load_workspace_operational_settings
---

# Настройки операции не меняются посреди её выполнения

Каждый вызов, которому нужны операционные настройки, получает свежий снимок
для своего рабочего пространства. Все сроки этого вызова берутся из одного
снимка: изменение файла действует со следующего вызова.

Ошибка загрузки останавливает операцию до запуска её поставщика. Если
во время загрузки поступила отмена, результатом становится отмена,
а не одновременно обнаруженная ошибка настроек.

Проверки проходят канонические search, BSL check и чтение графа через
подготовку, исполнение и публикацию актора. Полное повторное чтение большого
графа использует тот же снимок. Отдельная проверка настоящего MCP-сервера
подтверждает, что создание сервера, initialize и полный tools/list не
загружают настройки, даже если файл ошибочен или недоступен для чтения.
