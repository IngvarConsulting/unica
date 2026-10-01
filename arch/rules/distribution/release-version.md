---
id: INV.PKG.VERSION-LOCKSTEP
check:
  - tests/ci/test_version_contract.py::VersionContractTests.test_every_contract_location_declares_the_same_version
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_source_plugin_carries_a_manifest_for_every_host
---

# Все части описания выпуска указывают одну версию Unica

Версия Unica совпадает в Cargo workspace, манифестах плагина для Codex
и Claude Code и записи `unica` в `tools.lock.json`. Оба манифеста находятся
в одном исходном каталоге плагина. Версия имеет вид `1.2.3` или
`1.2.3-rc.1` для предвыпуска.
