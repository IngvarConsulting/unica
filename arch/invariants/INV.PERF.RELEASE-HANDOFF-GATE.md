---
id: INV.PERF.RELEASE-HANDOFF-GATE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_the_catalog_moves_only_behind_green_consumer_installs
scope: [ci, pkg, product]
---

# Каталог ждёт проверенной потребительской установки

Продвижение стабильного каталога зависит от проверки установленных байтов
обоими клиентами и не происходит после неуспешного потребительского шлюза.
