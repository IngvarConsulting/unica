---
id: INV.SOURCE.RETAINED-APPLY-TRANSIENT-ENTRY-AUTHORITY
status: active
governs: product
decision: DEC.2026-08-27.RETAINED-APPLY-REVISION-TRANSIENT-AUTHORITY-SLICE
check: crates/unica-coder/src/infrastructure/source_revision.rs::retained_apply_revision_transient_authority_preserves_projection_capture_bounds
scope: [app, cache, platform, source]
---

# Временный recovery entry исключается только authority журнала

Retained-apply projection и postpublication capture разделяют entry/depth
bounds. Scanner не учитывает только exact live single-link recovery entries,
доказанные sealed borrowed authority текущего journal; чужие ignored entries,
остатки cleanup и recoveries без полной identity-проверки расходуют обычный
лимит или закрывают validation с сохранением rollback evidence.
