---
id: CTR.SOURCE.REVISION-ARTIFACT-PROFILE
status: active
governs: product
decision: DEC.2026-08-27.ACTOR-REVISION-ARTIFACT-POLICY-SLICE
check: crates/unica-coder/src/infrastructure/revision_artifact_policy.rs::platform_xml_revision_artifact_profile_is_closed_and_legacy_is_unchanged
scope: [platform, source]
version: 1
producer: crates/unica-coder/src/infrastructure/revision_artifact_policy.rs
consumers: [platform, review]
---

# Профиль артефактов ревизии actor-owned source set

Legacy v0.12 индексирует прежний набор расширений. Actor Platform XML 8.3.27
format 2.20 добавляет только закрытые XDTO, support, template, help и form-item
пути; остальные файлы не получают manifest entry и не расходуют byte budget.

Контентные ресурсы входят путём, kind и digest. Прямые vendor `.cf` входят
путём, presence kind и retained identity без чтения payload.
