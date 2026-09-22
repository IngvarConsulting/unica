---
id: INV.SOURCE.OPTIONAL-HOME-PAGE
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_home_page_distinguishes_missing_from_malformed_and_wrong_root
---

# Отсутствие начальной страницы отличается от повреждённого описания

При чтении конфигурации отсутствующий `Ext/HomePageWorkArea.xml`
означает отсутствие описания начальной страницы. Если файл есть,
но его XML повреждён или имеет другой корневой элемент, чтение возвращает
`provider_unavailable`. Исправное описание передаёт свойства страницы.
