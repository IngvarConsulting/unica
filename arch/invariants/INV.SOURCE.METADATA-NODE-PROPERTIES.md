---
id: INV.SOURCE.METADATA-NODE-PROPERTIES
status: active
governs: product
decision: DEC.2026-09-10.METADATA-PROPERTIES-ARE-NODE-PROPS
check: crates/unica-coder/src/infrastructure/v13_read/tests.rs::metadata_node_props_carry_the_observed_object_properties
scope: [product, source]
---

# Узел объекта метаданных отвечает своими свойствами

`props` узла объекта метаданных несут наблюдаемые свойства объекта под их
ключами из закрытого словаря. Структурное значение приходит компактной
строкой. Проекция не ищет свойства среди пофактовых полей вида: там их нет.
