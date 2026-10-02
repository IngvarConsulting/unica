---
id: INV.PKG.OLDEST-CLIENT-KEYS
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_claude_contracts_avoid_keys_older_clients_reject
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_claude_manifest_leaves_skill_discovery_to_the_default_scan
---

# Манифест и каталог Claude используют ограниченный набор полей

Манифест плагина Claude допускает только `name`, `version`, `description`,
`author`, `homepage`, `repository`, `license` и `keywords`. Он не дублирует
стандартные места обнаружения навыков и MCP настройками `skills` или
`mcpServers`.

Запись плагина в каталоге может дополнительно содержать `source`, `category`
и `tags`. Корень каталога допускает только `name`, `owner`, `metadata`
и `plugins`; описание находится в `metadata.description`.

Связанные проверки читают манифест и создают каталог упаковщиком.
Они проверяют состав полей без запуска клиента Claude.
