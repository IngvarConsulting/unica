---
id: INV.SOURCE.FORMAT-PER-SET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/format_guard.rs::code_patch_inside_older_source_set_uses_the_same_format_boundary
scope: [source]
---

# Кодовая мутация соблюдает формат своего набора исходников

`unica.code.patch` внутри набора старого формата проходит тот же format guard,
что XML-мутации, и отказывает без изменения модуля.
