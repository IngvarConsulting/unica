---
id: INV.SOURCE.SUBSYSTEM-TOPOLOGY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::subsystem_projection_contract_is_complete
scope: [source]
---

# Публичные проекции подсистем выводятся из регистрации

`subsystem.info` публикует только дерево из зарегистрированной топологии: ответ
для выбранной подсистемы содержит цепочку её предков и всё зарегистрированное
поддерево, но не соседние ветви. Ответ одновременно сохраняет типизированные
`Content` и командный интерфейс, не следует по ссылкам и не заимствует
незарегистрированную раскладку; снятый `Mode` не выбирает состав результата.
