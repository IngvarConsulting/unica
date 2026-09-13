---
id: INV.SOURCE.OBSERVED-BYTES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_preserves_raw_bytes_and_excludes_one_bom_from_text
scope: [source]
---

# Снимок сохраняет исходные байты и отделяет BOM

Текстовый снимок сохраняет исходные байты без изменений и отделяет ровно один
UTF-8 BOM от строки, с которой работает редактор.
