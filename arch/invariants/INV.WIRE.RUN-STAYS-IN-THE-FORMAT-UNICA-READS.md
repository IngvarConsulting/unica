---
id: INV.WIRE.RUN-STAYS-IN-THE-FORMAT-UNICA-READS
status: active
governs: product
decision: DEC.2026-09-15.SOURCE-CONVERT-LEAVES-THE-DICTIONARY
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_names_no_operation_that_needs_edt
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_directional_runtime_intents
scope: [wire, product]
---

# В словаре `run` нет операции для формата, который Unica не читает

Unica читает и пишет выгрузку Designer. Операция словаря `run`, которой нужен
EDT, его CLI или перевод между форматами исходников, не публикуется: имя
операции в поверхности — адрес вызова, и адрес к отсутствующему инструменту
поверхность не печатает. Проверка перечисляет словарь целиком и падает на
имени с `convert` и на описании, обещающем EDT.
