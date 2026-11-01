---
id: CTR.SOURCE.LOGICAL-NODE-VIEW-SHAPE
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/xdto_imports.rs::xdto_import_pages_preserve_large_lists_and_one_indivisible_uri_without_addresses
  - crates/unica-coder/src/domain/node_view.rs::node_view_has_exactly_seven_common_slots_and_omits_empty_optional_slots
  - crates/unica-coder/src/domain/node_view.rs::only_a_collection_adds_items_and_data_rows_do_not_gain_addresses
---

# Логический узел отделяет свои данные от служебных полей ответа

У адресуемого узла семь общих допустимых полей: `at`, `kind`, `title`,
`props`, `branches`, `can`, `limits`. Пустые необязательные поля опускаются.
Только коллекция добавляет `items`; строки данных и исходного текста
не получают выдуманных адресов `at`.

Узел не включает `set`, `sourceState`, `fileExists`, физическую раскладку
или необработанный ответ поставщика. Признак успеха `ok`, итог `summary`,
диагностика, ревизия `rev` и курсор принадлежат внешнему результату
`DomainResult`, а не самому узлу.

Unica читает ресурс как предметные данные поддерживаемого вида.
Универсальная выдача точных байтов любого ресурса не входит в контракт.
Для исследования неподдержанного содержимого `resolve` помогает найти
файл, который затем открывают вне Unica. Это не ослабляет требования
сохранности исходных байтов при изменениях.
