---
id: INV.WIRE.RUN-NEEDS-THE-PLATFORM
status: active
governs: product
decision: DEC.2026-09-09.PROJECT-CONFIG-IS-HANDWRITTEN
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_directional_runtime_intents
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_operations_without_query_execution
scope: [wire, product]
---

# В словаре `run` нет операции, которая обходится без платформы

`run` отвечает за то, для чего нужна платформа 1С или информационная база.
Операция, которая только пишет файлы исходников, принадлежит `apply`;
операция, которая только читает узел, — `view`.

Граница без исключений: проверка перечисляет словарь целиком, и появление в
нём имени, которому платформа не нужна, роняет её. Проектный файл в этот
словарь не возвращается — его заводит человек.
