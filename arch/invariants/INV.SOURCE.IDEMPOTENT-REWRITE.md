---
id: INV.SOURCE.IDEMPOTENT-REWRITE
status: active
governs: product
decision: DEC.2026-08-21.MUTATION-IDEMPOTENCE-SCOPE
check: crates/unica-coder/src/infrastructure/native_operations/source_invariant_tests.rs::verified_public_mutator_idempotence_cases_are_exact
scope: [source]
---

# Повторный эквивалентный постобраз не заменяет файл

Точный набор из 12 публичных мутаторов имеет собственный повторный сценарий,
который сохраняет байты и идентичность файла. Это правило не распространяет
доказательство на остальные мутаторы и не задаёт форму публичной квитанции.
