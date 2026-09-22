---
id: INV.WIRE.RUN-NAMES-ARE-LAYER-AND-DIRECTION
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::runner_one_vocabulary_replaces_the_previous_public_dictionary
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_names_no_operation_that_needs_edt
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_operations_without_query_execution
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_run_dictionary_returns_directly_before_workspace_actor_admission
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
исполнения, `implemented` и `support`. Для `previewApply` обязательны preview
и его `ifRev`. Доступность адаптера не означает готовность среды запуска.
