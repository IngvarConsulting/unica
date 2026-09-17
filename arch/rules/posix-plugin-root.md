---
id: INV.PKG.POSIX-TWO-HOSTS-ONE-ROOT
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_packaged_alias_resolves_the_plugin_root_for_both_hosts
---

# POSIX launcher разрешает один корень плагина

Упакованный POSIX launcher разрешает один корень плагина при запуске из
каталога плагина по соглашению Codex и из каталога маркетплейса с подставленным
корнем Claude Code. Рабочий каталог хоста не меняет выбранный плагин.
