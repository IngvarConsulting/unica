---
id: INV.SOURCE.LOGICAL-IDENTITY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/domain/source_target.rs::source_target_profile_emits_canonical_english_kind_tokens
  - crates/unica-coder/src/domain/source_target.rs::source_target_profile_normalizes_only_registered_exact_russian_kind_aliases
  - crates/unica-coder/src/domain/source_target.rs::source_target_profile_preserves_application_name_case
  - crates/unica-coder/src/domain/source_target.rs::source_target_and_resolved_target_serialize_only_logical_identity
scope: [source]
---

# Точная цель не зависит от файловой раскладки

Точная существующая цель сериализуется именем `sourceSet` и необязательным
каноническим `metadataPath`: английские и русские виды нормализуются в
английские токены, прикладные имена сохраняют регистр, а разрешённая цель не
возвращает физический путь как свою идентичность.
