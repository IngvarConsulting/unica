---
id: INV.SOURCE.LOGICAL-IDENTITY
check:
  - crates/unica-coder/src/domain/source_target.rs::source_target_profile_emits_canonical_english_kind_tokens
  - crates/unica-coder/src/domain/source_target.rs::source_target_profile_normalizes_only_registered_exact_russian_kind_aliases
  - crates/unica-coder/src/domain/source_target.rs::source_target_profile_preserves_application_name_case
  - crates/unica-coder/src/domain/source_target.rs::source_target_and_resolved_target_serialize_only_logical_identity
---

# Разрешённая цель исходников сохраняет логическую идентичность

Внутри разрешения исходников `SourceTarget` и `ResolvedTarget` обозначают
цель через имя набора `sourceSet` и необязательный адрес `metadataPath`.
Физическое расположение файла не включается в эту идентичность.

Виды объектов приводятся к каноническим английским именам; принимаются
только зарегистрированные русские псевдонимы. Регистр прикладных имён
сохраняется: например, `CommonModule.eBayHTTP.Module` не переименовывает
модуль. Этот внутренний контракт отличается от
[публичного адреса at](../../mcp/qualified-logical-address.md).
