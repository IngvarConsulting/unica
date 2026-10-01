---
id: INV.APP.CODE-SEARCH-SOURCE-BOUNDARIES
check:
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::bsl_analyzer_does_not_broaden_metadata_scoped_search
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::rlm_does_not_broaden_metadata_scoped_search
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::git_grep_excludes_the_generated_cache_from_the_lexical_corpus
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::git_grep_does_not_publish_a_parent_escape_as_a_location
---

# Поставщик не расширяет неподдерживаемую область поиска

Если `bsl-analyzer` или RLM не может выполнить поиск в заданной области
метаданных, адаптер отказывает до вызова клиента. Он не заменяет такой запрос
поиском по всему рабочему пространству.

Буквальный поиск `git-grep` исключает служебные каталоги `.build` на любой
глубине: их содержимое не попадает в выдачу и не расходует лимит вместо
исходников. Кандидат, выходящий через `..` за корень исходников, не публикуется
как местоположение найденного кода.

Проверки подтверждают эти конкретные границы адаптеров; они не доказывают
поддержку всех возможных областей каждым движком.
