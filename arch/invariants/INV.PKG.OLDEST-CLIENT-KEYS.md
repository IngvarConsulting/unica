---
id: INV.PKG.OLDEST-CLIENT-KEYS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_claude_contracts_avoid_keys_older_clients_reject
scope: [pkg, product]
---

# Claude-контракты не используют новые необязательные ключи

Манифест и каталог Claude содержат только наборы ключей, принимаемые нижней
поддерживаемой версией клиента.
