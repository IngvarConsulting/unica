---
id: INV.WIRE.REFUSAL-CODE-NAMES
check:
  - crates/unica-coder/src/domain/refusal.rs::the_dictionary_keeps_no_synonym_of_a_code_it_already_carries
---

# Снятые имена отказов не возвращаются в публичный словарь

В каноническом словаре используются `stale_revision`, `deadline_exceeded`
и `provider_unavailable`. Их снятые синонимы `revision_mismatch`,
`provider_deadline` и `dependency_unavailable` не публикуются.

Проверка закрепляет именно эти три пары имён. Она не определяет
автоматически смысловую эквивалентность любых будущих кодов.
