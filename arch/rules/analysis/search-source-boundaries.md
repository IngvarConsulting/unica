---
id: INV.APP.CODE-SEARCH-SOURCE-BOUNDARIES
check:
  - crates/unica-coder/tests/platform/code_intelligence_integration.rs::bsl_analyzer_does_not_broaden_metadata_scoped_search
  - crates/unica-coder/tests/platform/code_intelligence_integration.rs::rlm_does_not_broaden_metadata_scoped_search
  - crates/unica-coder/tests/platform/code_intelligence_integration.rs::git_grep_excludes_the_generated_cache_from_the_lexical_corpus
  - crates/unica-coder/tests/platform/code_intelligence_integration.rs::git_grep_does_not_publish_a_parent_escape_as_a_location
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_search_is_source_scoped_and_rejects_legacy_call_shape
---

# Поставщик не расширяет неподдерживаемую область поиска

Если `bsl-analyzer` или RLM не может выполнить поиск в заданной области
метаданных, адаптер отказывает до вызова клиента. Он не заменяет такой запрос
поиском по всему рабочему пространству.

Буквальный поиск исключает служебные каталоги `.build` на любой глубине и
корни вложенных наборов исходников, которые читает их собственный набор:
их содержимое не попадает в выдачу. Кандидат, выходящий через `..` за корень исходников, не публикуется
как местоположение найденного кода.

Проверки подтверждают эти конкретные границы адаптеров; они не доказывают
поддержку всех возможных областей каждым движком.
