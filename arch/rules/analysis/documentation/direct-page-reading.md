---
id: INV.APP.DOCUMENTATION-DIRECT-PAGE
check:
  - crates/unica-documentation-sources/src/platform_help/provider.rs::get_reads_only_the_requested_entry_and_survives_a_corrupted_sibling
---

# Открытие страницы справки не требует чтения остальных страниц

По локатору страницы установленной справки Unica читает нужную запись
контейнера: повреждённая соседняя запись не мешает открыть исправную.
