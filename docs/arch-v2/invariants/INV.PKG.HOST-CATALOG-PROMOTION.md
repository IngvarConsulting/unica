---
id: INV.PKG.HOST-CATALOG-PROMOTION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_publish_workflow_promotes_both_host_catalogs
scope: [host, pkg, product]
---

# Продвижение переносит оба host-каталога вместе

Публикация копирует и индексирует записи каталогов Codex и Claude Code одним
продвигаемым изменением.
