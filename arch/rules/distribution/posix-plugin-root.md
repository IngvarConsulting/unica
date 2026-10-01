---
id: INV.PKG.POSIX-TWO-HOSTS-ONE-ROOT
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_packaged_alias_resolves_the_plugin_root_for_both_hosts
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_packaged_alias_launches_with_zcode_expansion_from_another_cwd
---

# Codex, Claude Code и ZCode находят один и тот же установленный плагин

Поставляемый скрипт запуска для POSIX-систем, таких как Linux и macOS,
должен находить один и тот же каталог плагина в следующих случаях:

- Codex запускает его из каталога самого плагина;
- Claude Code запускает его из каталога маркетплейса, подставив путь к плагину;
- ZCode подставляет совместимый `CLAUDE_PLUGIN_ROOT` и запускает его вне
  каталога плагина, в том числе если путь содержит пробелы и кириллицу.

Разница в начальном каталоге не должна приводить к запуску другого плагина.
Windows этой проверкой не охвачен.
