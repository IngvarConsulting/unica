---
id: CTR.SOURCE.MODULE-PROJECTION-SHAPE
status: active
governs: product
decision: DEC.2026-08-25.MODULE-PROJECTION-CORE-SLICE
check: crates/unica-coder/src/domain/module_projection.rs::serialized_module_projection_shape_is_stable
scope: [product, source]
version: 1
producer: crates/unica-coder/src/domain/module_projection.rs
consumers: [platform, review]
---

# Сериализованная форма скрытой проекции модуля v0.13

Сводка модуля содержит квалифицированный `at`, `kind: Module`, `title`,
предметные `props`, ровно шесть упорядоченных веток `Method`, `Region`,
`Interface`, `Event`, `Compilation`, `Body` с фактическими счётчиками и `rev`.
Она не содержит методы или исходный текст. Допустимый модуль без файла имеет
нулевые source-derived ветки, но сохраняет применимые possible events.

Восемь нормализованных свойств общего модуля появляются один раз в его
`props`; `serverCall`, `privileged` и `returnValuesReuse` не становятся
контекстами. Каноническая форма не содержит `set`, `sourceState`, `fileExists`
или универсальную роль `Service`.
