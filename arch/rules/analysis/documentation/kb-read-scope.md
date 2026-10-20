---
id: INV.APP.DOCUMENTATION-KB-READ-SCOPE
check:
  - crates/unica-documentation-sources/src/kb_1ci.rs::kb_get_refuses_a_locator_with_parent_segments
  - crates/unica-documentation-sources/src/kb_1ci.rs::kb_get_refuses_encoded_or_ambiguous_path_segments_before_network
  - crates/unica-documentation-sources/src/kb_1ci.rs::kb_get_preserves_encoded_page_names_inside_a_guide
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::docs_refuses_kb_locators_outside_the_complete_guide_roots_before_network
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::docs_opens_both_kb_guide_roots_with_short_and_long_version_aliases
  - crates/unica-documentation-sources/src/kb_1ci.rs::kb_provider_matches_titles_and_reads_only_the_top_pages
gap: https://github.com/IngvarConsulting/unica/issues/953
---

# База знаний читает объявленные руководства с ограничением обращений

База знаний читает объявленные руководства разработчика и администратора.
Адреса за пределами этих руководств и выход через `..` отклоняются.
Поиск берёт кандидатов из дерева руководств, а не из произвольных ссылок.

Число дочитываемых страниц ограничено. Остальные совпадения сохраняют
заголовок и локатор без фрагмента; обращения разнесены по времени.

Проверки подтверждают отказ за пределами корня и для опасных сегментов без
сети, чтение допустимых версионных локаторов и страниц совпадений
в контролируемом дереве. Предел числа страниц и интервал обращений
остаются в `gap`.
Запрет и отмена действуют по [правилу сетевого чтения](network-read-boundaries.md).
