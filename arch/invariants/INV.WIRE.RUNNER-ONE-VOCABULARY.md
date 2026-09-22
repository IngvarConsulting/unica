---
id: INV.WIRE.RUNNER-ONE-VOCABULARY
status: active
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check: crates/unica-coder/src/application/v13/tool_catalog.rs::runner_one_vocabulary_replaces_the_previous_public_dictionary
scope: [wire, app, product]
---

# Словарь runtime использует целевые имена раннера 1.0

`unica.run` публикует имена целевого раннера 1.0; подкоманды разделены точкой.
Предыдущий словарь Unica не является набором aliases. Объявленный каталог
может включать недоступные режимы только с явно опубликованной доступностью;
он не является произвольным исполнителем CLI. `unica.apply` правит исходники,
а `unica.run` с `op: apply` относится к конфигурации базы данных.
