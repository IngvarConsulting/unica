---
id: INV.SOURCE.ENTITY-SPELLED-VERSION
status: active
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: crates/unica-coder/src/application/mod.rs::entity_spelled_supported_format_is_invalid_at_the_public_boundary
scope: [source]
---

# XML-сущности не канонизируют литерал версии

Сырой лексический срез атрибута `version` проверяется до декодирования
XML-сущностей. Написания `2.&#50;0`, `&#x32;.20` и `2.2&#48;` отклоняют мутацию
без изменения байтов, событий или артефактов. Совет о формате при чтении даёт
канонический путь проверки узла, а не сам читатель.
