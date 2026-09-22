---
id: INV.APP.DOCUMENTATION-DIRECT-PAGE
check:
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::get_reads_only_the_requested_entry_and_survives_a_corrupted_sibling
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_get_opens_the_page_by_its_pretty_locator_without_walking_the_tree
---

# Открытие страницы справки не требует чтения остальных страниц

По локатору страницы установленной справки Unica читает нужную запись
контейнера: повреждённая соседняя запись не мешает открыть исправную.

Страница базы знаний открывается прямо по локатору, без обхода
навигационного дерева.
