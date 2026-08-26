---
id: INV.WIRE.CODE-PATCH-REPLACEMENT-BATCH
status: active
governs: product
decision: DEC.2026-08-22.ATOMIC-CODE-REPLACEMENT-BATCH
check: crates/unica-coder/src/application/tool_contracts.rs::code_patch_json_schema_accepts_each_documented_selector_variant
scope: [wire]
---

# Code patch публикует две формы replace

Закрытая схема `unica.code.patch` принимает прежнюю плоскую замену или пакет
`replacements` из элементов `selector`, `content`, `expectedCount` и не
разрешает смешивать эти формы.
