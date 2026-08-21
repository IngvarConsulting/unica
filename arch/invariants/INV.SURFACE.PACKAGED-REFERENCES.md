---
id: INV.SURFACE.PACKAGED-REFERENCES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_all_active_packaged_documentation_links_are_relative_and_resolve
scope: [pkg]
---

# Ссылки документации переживают упаковку

После копирования отслеживаемого дерева плагина каждая относительная ссылка
между поставляемыми Markdown-документами разрешается внутри пакета.
