---
id: INV.PKG.ATTRIBUTION
check:
  - tests/ci/test_attributions.py::AttributionTests.test_repository_attribution_page_is_complete_and_linked
  - tests/ci/test_attributions.py::AttributionTests.test_validation_reports_missing_unknown_and_invalid_repository_links
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_attribution_page_and_referenced_local_licenses_are_packaged
---

# Пакет называет источники заимствований и содержит локальные лицензии

В поставляемом `ATTRIBUTIONS.md` перечислены проект, внешние инструменты,
подключённые сервисы и источники заимствований из манифестов и данных
происхождения. Лишняя или пропущенная запись не проходит проверку.

Для проекта и инструментов указаны репозиторий, автор и заявленная лицензия
со ссылкой на файл внутри пакета. Для остальных источников допустима внешняя
ссылка на лицензию; она не обязательна, если источник использован только
как идея (`inspiration-only`). Внешний сервис обозначен как не входящий
в поставку.

Проверка упаковки подтверждает наличие страницы и указанных в ней локальных
файлов с `LICENSE` в пути. Проверки сверяют состав записей, ссылки и наличие
файлов; юридическую полноту текста они не устанавливают.
