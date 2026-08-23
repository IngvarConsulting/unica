---
id: DEC.2026-08-21.SOURCE-READ-ONLY-SURFACE
status: active
governs: product
realized: crates/unica-coder/src/application/mod.rs::source_resource_tools_are_read_only_and_have_no_cache_or_event_effects
supersedes: []
superseded-by: null
establishes: [INV.SURFACE.SOURCE-TOOL-SPECS]
---

# Ресурсная поверхность source остаётся только для чтения

**Решение.** Публичная ресурсная поверхность состоит из
`unica.source.resources` и `unica.source.read`; `unica.source.apply` в ней нет.
Целевое изменение BSL выполняет `unica.code.patch`. Этим решением явно
замещается прежняя часть требования v1 про полную замену до 1 МиБ: оставшаяся
неиспользуемая константа не считается действующим ограничением продукта.

**Почему.** Реестр, схемы и обработчики уже не содержат общей замены ресурса, а
перенос числового предела без достижимого публичного вызова создавал бы ложное
обещание совместимости.

**Цена.** Клиент прежнего `source.apply` должен выбрать предметный writer;
универсальная полная замена модуля намеренно не возвращается.
