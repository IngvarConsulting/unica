---
id: INV.PKG.PLATFORM-DOCUMENTATION-CORPUS
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_plugin_source_copy_excludes_vendor_corpus_outside_the_plugin_root
gap: https://github.com/IngvarConsulting/unica/issues/953
---

# Материалы справки платформы не копируются в Unica

Разбор справки установленной платформы не записывает контейнеры, индекс
и распакованные страницы на диск. Материалы вендора не копируются
в репозиторий и не входят в пакет.

Проверка упаковки подтверждает исключение корпуса вне корня плагина.
Она не доказывает общий запрет копирования в репозиторий или пакет
и отсутствие записей при работе поставщика; эти границы остаются в `gap`.
