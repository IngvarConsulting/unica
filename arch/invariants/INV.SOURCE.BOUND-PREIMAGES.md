---
id: INV.SOURCE.BOUND-PREIMAGES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations.rs::public_platform_xml_mutator_preimage_contract_is_complete
scope: [source]
---

# Мутация привязана к байтам, из которых выведена

Закрытая таблица сопоставляет каждому из 25 публичных XML-мутаторов отдельный
сценарий фактического владельца, выведенного входа или проверки отсутствия.
Каждый сценарий связывает байты либо отсутствие с транзакцией, поэтому дрейф
между планированием и публикацией отклоняет изменение без перезаписи конкурента.
