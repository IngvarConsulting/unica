---
id: INV.WIRE.RUN-NAMES-ARE-LAYER-AND-DIRECTION
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::runner_one_vocabulary_replaces_the_previous_public_dictionary
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_names_no_operation_that_needs_edt
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_operations_without_query_execution
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_run_dictionary_returns_directly_before_workspace_actor_admission
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_run_rejects_if_rev_before_workspace_or_provider_admission
  - crates/unica-coder/src/application/v13/tool_catalog.rs::run_schema_accepts_direct_execution_and_rejects_revision_arguments
---

# Словарь run использует целевые имена раннера 1.0

`unica.run {}` до допуска исходников возвращает закрытый каталог операций.
Подкоманды разделены точкой: `infobase.dump`, `extensions.set`.
Прежние имена Unica не принимаются как синонимы. Каталог не разрешает
произвольные команды CLI, создание проектного файла, выполнение запросов
или преобразование Designer ↔ EDT.

Каталог различает исходники (`push`/`pull`), пакеты (`upload`/`download`,
`make`) и базу целиком (`infobase.dump`/`infobase.restore`). `unica.apply`
изменяет исходники; `unica.run` с `op: apply` изменяет конфигурацию БД.

Каждая операция публикует назначение, схему аргументов, эффекты, режим
исполнения, `implemented` и `support`. Режим `previewApply` требует явный
boolean `dryRun`: `true` выбирает preview, `false` исполняет текущий запрос
без предшествующего просмотра. Каталог сообщает `dryRunRequired: true`
и `previewRequired: false`. У терминального `launch` `dryRun` необязателен
и `dryRunRequired: false`; его поведение задаёт [контракт запуска](../runtime/client-launch.md).

`run` не принимает `ifRev` и не выдаёт `rev`. Отдельный preview не закрепляет
аргументы, входные файлы, проект или поколение базы между вызовами.
Исполнение проверяет текущий запрос и входы; предметные проверки внутри
вызова задаёт правило операции. Доступность адаптера не означает готовность
среды запуска.
