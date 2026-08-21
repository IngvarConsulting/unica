---
id: INV.SOURCE.LOGICAL-IDENTITY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/source_target.rs::source_target_and_resolved_target_serialize_only_logical_identity
scope: [source]
---

# Точная цель не зависит от файловой раскладки

Точная существующая цель задаётся именем `sourceSet` и необязательным
каноническим `metadataPath`: английские и русские виды нормализуются в
английские токены, прикладные имена сохраняются, а физический путь не
принимается и не возвращается как идентичность цели.
