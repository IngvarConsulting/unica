---
id: INV.APP.OUTLINE-SOURCE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/tests/platform/code_intelligence_symlinked_workspace.rs::code_outline_answers_from_the_current_file_without_touching_the_index
scope: [app]
---

# Outline читает текущий файл без индекса

`unica.code.outline` возвращает типизированную структуру текущего BSL-файла без
`stdout`, не объявляет `bsl_index` свежим и не создаёт каталог состояния
индекса.
