---
id: INV.WIRE.SEARCH-CORPUS
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::search_publishes_exactly_two_corpora_and_defaults_to_text
---

# Схема поиска различает текст и имена

Аргумент `corpus` у `unica.search` принимает только `text` и `names`.
Опубликованное умолчание — `text`: отсутствие аргумента сохраняет текстовый
поиск. Проверка закрепляет схему, которую получает клиент.
