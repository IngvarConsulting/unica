---
id: INV.SOURCE.FORMAT-PER-SET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/format_guard.rs::code_patch_inside_older_source_set_uses_the_same_format_boundary
scope: [source]
---

# Формат — свойство набора исходников

Формат выгрузки принадлежит набору, а не рабочему пространству: в одном пространстве могут
действовать несколько форматов, но один набор не бывает двух форматов сразу. Нативные
операции с XML требуют доказанного platform XML и отказывают иначе.
