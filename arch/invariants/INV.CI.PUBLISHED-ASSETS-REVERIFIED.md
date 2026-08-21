---
id: INV.CI.PUBLISHED-ASSETS-REVERIFIED
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_release_assets_are_published_without_pages_dependency_and_redownloaded
scope: [ci, pkg]
---

# Опубликованные байты скачиваются и проверяются повторно

Релизный workflow заново скачивает опубликованные архив и метаданные ядра и
передаёт их проверке выпускных ассетов.
