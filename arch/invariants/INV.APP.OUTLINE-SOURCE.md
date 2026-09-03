---
id: INV.APP.OUTLINE-SOURCE
status: superseded
governs: product
decision: DEC.2026-09-03.V0-13-LEGACY-BATCH-2
check: crates/unica-coder/tests/platform/v13_canonical_symlinked_workspace.rs::canonical_stdio_views_a_module_through_a_symlinked_workspace
scope: [app]
---

# Outline читает текущий файл без индекса

`unica.code.outline` возвращает типизированную структуру текущего BSL-файла без
`stdout`, не объявляет `bsl_index` свежим и не создаёт каталог состояния
индекса.
