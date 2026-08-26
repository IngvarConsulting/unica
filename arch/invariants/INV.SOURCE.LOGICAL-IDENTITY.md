---
id: INV.SOURCE.LOGICAL-IDENTITY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/source_target.rs::logical_target_identity_contract_is_complete
scope: [source]
---

# Точная цель не зависит от файловой раскладки

Точная существующая цель сериализуется именем `sourceSet` и необязательным
каноническим `metadataPath`: английские и русские виды нормализуются в
английские токены, прикладные имена сохраняют регистр, а разрешённая цель не
возвращает физический путь как свою идентичность.
