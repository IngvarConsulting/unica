---
id: INV.SOURCE.BOUND-PREIMAGES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations.rs::public_platform_xml_mutator_preimage_contract_is_complete
scope: [source]
---

# Мутация привязана к байтам, из которых выведена

Закрытый реестр относит все 25 публичных XML-мутаторов к 13 семействам. Каждое
семейство повторно проверяет фактического владельца или иной выведенный вход и
связывает его байты либо отсутствие с общей транзакцией, поэтому дрейф между
планированием и публикацией отклоняет изменение без перезаписи конкурента.
