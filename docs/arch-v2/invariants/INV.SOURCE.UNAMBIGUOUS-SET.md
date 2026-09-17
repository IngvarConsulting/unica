---
id: INV.SOURCE.UNAMBIGUOUS-SET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/project_sources.rs::conflicting_markers_inside_one_source_set_are_invalid_not_mixed
scope: [source]
---

# Один набор исходников не бывает двух форматов сразу

Противоречащие друг другу признаки формата внутри одного набора исходников
делают его недопустимым или неоднозначным; набор никогда не сообщает смешанный
формат.
