---
id: INV.SOURCE.SUBSYSTEM-TOPOLOGY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::public_subsystem_projection_and_mode_absence_contract_is_complete
scope: [source]
---

# Публичные проекции подсистем выводятся из регистрации

Публичный `subsystem.info` не имеет поля `Mode`, а его типизированный обработчик
строит дерево только из зарегистрированной топологии: выбранная подсистема
содержит цепочку предков и зарегистрированное поддерево, сохраняет `Content` и
командный интерфейс, не следует по ссылкам и не заимствует незарегистрированную
раскладку. Ошибка или срок не публикуют данные.
