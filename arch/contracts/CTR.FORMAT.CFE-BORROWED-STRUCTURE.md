---
id: CTR.FORMAT.CFE-BORROWED-STRUCTURE
status: active
governs: product
version: 1
decision: DEC.2026-09-14.CFE-BORROWED-STRUCTURE
producer: crates/unica-coder/src/infrastructure/native_operations/cfe.rs
consumers: [platform, review]
check: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_checks_borrowed_cfe_structure_and_reports_borrowing_unavailable
scope: [platform, wire]
---

# Проверка CFE различает пустую коллекцию и повреждённую структуру

На заимствованном `Report` проверка корня расширения требует единственный
`ChildObjects` в namespace `http://v8.1c.ru/8.3/MDClasses` после `Properties`.
Пустой контейнер и произвольный префикс того же namespace допустимы;
отсутствующий, повторный, чужой или переставленный контейнер дают
`data.status=failed`, `validators=[cfe]` и диагностику с `validator=cfe`.

При существующем `Ext/ObjectModule.bsl` требуется единственный
`xr:PropertyState` с последовательностью `xr:Property=ObjectModule`,
`xr:State=Extended` в namespace `http://v8.1c.ru/8.3/xcf/readable`.
Отсутствие, чужой namespace, обратный порядок и несколько `State`
отвергаются. Проверка не меняет дескриптор.
