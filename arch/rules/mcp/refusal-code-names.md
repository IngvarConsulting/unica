---
id: INV.WIRE.REFUSAL-CODE-NAMES
---

# Снятые имена отказов не возвращаются в публичный словарь

В каноническом словаре используются `stale_revision`, `deadline_exceeded`
и `provider_unavailable`. Их снятые синонимы `revision_mismatch`,
`provider_deadline` и `dependency_unavailable` не публикуются.

Проверка закрепляет именно эти три пары имён. Она не определяет
автоматически смысловую эквивалентность любых будущих кодов.
