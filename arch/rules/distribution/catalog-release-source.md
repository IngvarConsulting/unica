---
id: INV.PKG.CODEX-CATALOG-RELEASE-PIN
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_generated_marketplace_is_thin_pinned_and_target_neutral
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_claude_catalog_pins_the_release_tag
---

# Каталоги обоих хостов указывают на один выпуск плагина

При публичной упаковке каталоги Codex и Claude Code получают источник
`git-subdir`: репозиторий `IngvarConsulting/unica-marketplace`, подкаталог
`plugins/unica` и переданный тег выпуска. Оба хоста устанавливают плагин
из этого тега; изменения основной ветки не меняют выбранный выпуск.
