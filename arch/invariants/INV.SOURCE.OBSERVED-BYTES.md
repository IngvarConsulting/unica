---
id: INV.SOURCE.OBSERVED-BYTES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_preserves_raw_bytes_and_excludes_one_bom_from_text
scope: [source]
---

# Байты источника наблюдаются, а не назначаются

Перевод строки, BOM и прочие свойства текста берутся из самого файла, а не из умолчания
инструмента. Мутация привязана к тем байтам, из которых выведена: изменившийся источник
отклоняет план, а не переписывается по устаревшему снимку.
