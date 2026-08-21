---
id: INV.SOURCE.IDEMPOTENT-REWRITE
status: active
governs: product
decision: DEC.2026-08-21.MUTATION-IDEMPOTENCE-SCOPE
check: crates/unica-coder/src/infrastructure/native_operations.rs::public_platform_xml_mutator_idempotence_contract_is_complete
scope: [source]
---

# Повторный эквивалентный постобраз не заменяет файл

Все 25 публичных нативных и типизированных мутаторов platform XML принадлежат
одному из 13 закрытых семейств. Представительный повтор каждого семейства
сохраняет байты, а общий транзакционный писатель не заменяет файл при
эквивалентном постобразе; форма публичной квитанции регулируется отдельно.
