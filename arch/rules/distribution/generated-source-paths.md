---
id: INV.PKG.TRACKED-IGNORED-REJECTED
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_plugin_source_copy_rejects_tracked_source_bin
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_plugin_source_copy_rejects_tracked_nested_ignored_dir
---

# Генерируемые файлы в исходниках останавливают упаковку

При копировании отслеживаемых исходников плагина упаковщик отклоняет путь,
если среди его частей есть `bin`, `__pycache__`, `.pytest_cache` или
`.DS_Store`. Правило действует и во вложенных каталогах. Такой файл вызывает
отказ упаковки, а не молча пропускается.
