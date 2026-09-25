---
id: INV.WIRE.SEARCH-CORPUS
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::search_publishes_exactly_two_corpora_and_defaults_to_text
  - crates/unica-coder/src/application/v13/find.rs::find_returns_address_facts_and_not_content_hits
  - crates/unica-coder/src/application/v13/find.rs::exact_kind_alias_filter_limit_and_nearest_are_deterministic
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_search_is_source_scoped_and_rejects_legacy_call_shape
  - crates/unica-coder/tests/v13_search_integration.rs::names_search_pages_all_ranked_matches_and_rejects_changed_answers
---

# Поиск имён и текста показывает разные сведения о совпадении

`corpus: names` ищет имена и синонимы метаданных, `text` — текст модулей BSL.
Допустимы только эти два значения; без аргумента выбирается `text`.

Попадание по имени содержит логический адрес `at`, вид `kind` и название
`title`. Фильтр `kind` оставляет один вид узлов; `approximate` явно отмечает
совпадение по близости имени. Это не признак полноты поиска.

Попадание в тексте содержит `scope`, `line`, `column` и `snippet`.
Физический путь не выдаётся ни в одном своде. Текстовый результат связан
с прочитанной ревизией `rev`; справочник имён использует раскладку исходников
и сам по себе не обещает чтения на ревизии. Курсор поиска имён связан с
полным упорядоченным публичным ответом и полнотой чтения дескрипторов:
если что-либо из них изменилось, продолжение отвечает `stale_cursor`.

Проверки покрывают схему, внутренний справочник, публичные страницы имён и
области основного текстового поиска через MCP.
