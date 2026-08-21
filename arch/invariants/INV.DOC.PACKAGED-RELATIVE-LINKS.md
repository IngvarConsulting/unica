---
id: INV.DOC.PACKAGED-RELATIVE-LINKS
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_documented_resources_are_packaged
scope: [docs, pkg]
---

# Ссылки в документах пакета разрешаются после упаковки

Каждая относительная Markdown-ссылка и каждый документированный путь к
Markdown-файлу внутри плагина разрешаются от документа в скопированном пакете.
