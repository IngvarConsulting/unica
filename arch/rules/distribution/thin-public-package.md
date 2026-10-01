---
id: INV.PKG.THIN-PACKAGE
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_generated_marketplace_is_thin_pinned_and_target_neutral
---

# Публичный пакет содержит загрузчик, а ядро и движки доставляет отдельно

Публичный Git-пакет содержит файлы плагина, `runtime-manifest.json`
с описанием поставок и три нативных загрузчика `unica-bootstrap`:
для macOS arm64, Linux x64 и Windows x64. Каталог `bin` с ядром и движками
в этот пакет не входит; их поставки указаны в манифесте.

Внутренние материалы сопровождения, копии донорских скриптов для тестов
и отладочные архивы также не входят в публичный пакет.
