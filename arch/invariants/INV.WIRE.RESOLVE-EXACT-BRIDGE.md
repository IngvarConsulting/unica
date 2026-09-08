---
id: INV.WIRE.RESOLVE-EXACT-BRIDGE
status: active
governs: product
decision: DEC.2026-09-08.RESOLVE-REPLACES-FIND
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/application/v13/resolve.rs::resolve_takes_exactly_one_side_of_the_bridge
  - crates/unica-coder/src/application/v13/resolve.rs::absent_lines_are_named_rather_than_omitted
scope: [wire, product]
---

# Путь живёт в одном инструменте и отвечает точно

`resolve` принимает ровно одну сторону моста — `at` или `path`, — и отвечает
одним предметом либо `not_found`. Ранжированных кандидатов, признака близости
и поля с причиной совпадения у него нет: догадки принадлежат `search`.

Ни один другой предметный инструмент не публикует `path` на входе и не несёт
физического пути в ответе. Это и есть смысл отдельного инструмента: путь в
частом ответе звал бы читать файл мимо адреса.

Наличие строк называется закрытым признаком `lines.state`, а не отсутствием
поля. Диапазон обещается там, где источник строчный; там, где он древовидный,
признак говорит об этом прямо.
