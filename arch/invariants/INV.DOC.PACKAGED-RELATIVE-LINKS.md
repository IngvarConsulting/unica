---
id: INV.DOC.PACKAGED-RELATIVE-LINKS
status: active
governs: process
decision: DEC.2026-08-21.V2-PROCESS-POLICY
check: tests/ci/test_package_unica_plugin.py::test_all_active_packaged_documentation_links_are_relative_and_resolve
scope: [docs, pkg]
---

# Ссылки в документах пакета разрешаются после упаковки

Каждая относительная Markdown-ссылка и каждый документированный путь к
Markdown-файлу внутри плагина разрешаются от документа в скопированном пакете.
