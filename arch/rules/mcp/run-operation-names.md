---
id: INV.WIRE.RUN-NAMES-ARE-LAYER-AND-DIRECTION
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_names_read_as_layer_and_direction
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_directional_runtime_intents
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_names_no_operation_that_needs_edt
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_operations_without_query_execution
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_run_dictionary_returns_directly_before_workspace_actor_admission
---

# Словарь run называет платформенную операцию и направление данных

`unica.run {}` доступен до допуска исходников и возвращает закрытый словарь
операций. Имя имеет вид `<слой>.<глагол>`: слои — `infobase`, `cf`, `source`,
`artifact`, `client`; глаголы — `create`, `export`, `import`, `build`, `run`.
`export` направлен из базы наружу, `import` — снаружи в базу. Для каждого
из трёх слоёв переноса эти операции образуют пару.

Словарь содержит операции, которым нужна платформа 1С или информационная
база. Создания проектного файла, выполнения запросов и преобразования
Designer ↔ EDT в нём нет. Снятые имена с `dump`, `restore`, `load`
и `convert` не служат альтернативными именами публичных операций.
Точный состав проверяется по исполняемому каталогу; историческое число
в именах проверок не задаёт число операций.

Публичный результат сохраняет из каталога назначение, схему аргументов,
режим исполнения, эффекты и признак реализации каждой операции.
Для `previewApply` он указывает обязательность preview и `ifRev` при
применении. Проверка связывает каталог с JSON-результатом текущего V5 bind
до допуска исходников.
