---
id: INV.CI.THIN-PAYLOAD-RETENTION
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_thin_payload_downloads_only_metadata_and_bootstrap
scope: [ci, pkg]
---

# Тонкий marketplace-артефакт хранится для продвижения

Тонкая полезная нагрузка собирается из метаданных и bootstrap без полного
runtime и сохраняется девяносто дней для размещения в маркетплейсе.
