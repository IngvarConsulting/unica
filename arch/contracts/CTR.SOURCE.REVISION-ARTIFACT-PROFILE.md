---
id: CTR.SOURCE.REVISION-ARTIFACT-PROFILE
status: active
governs: product
decision: DEC.2026-08-27.ACTOR-REVISION-ARTIFACT-POLICY-SLICE
check: crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_artifact_policy_contract_is_complete
scope: [platform, source]
version: 1
producer: crates/unica-coder/src/infrastructure/revision_artifact_policy.rs
consumers: [platform, review]
---

# Профиль артефактов ревизии actor-owned source set

Legacy v0.12 индексирует прежний набор расширений. Actor Platform XML 8.3.27
format 2.20 добавляет только закрытые XDTO, support, template, help и form-item
пути; остальные файлы не получают manifest entry и не расходуют byte budget.
Для configuration/extension путь ресурса начинается с корня известной
коллекции и её непосредственного owner, для external processor/report — с
единственного непосредственного owner. Произвольный префикс или смешанная
цепочка `Forms`/`Templates` не совпадает с профилем.

Контентные ресурсы входят путём, kind и digest. Прямые vendor `.cf` входят
путём, presence kind и retained identity без чтения payload.
Ambient, retained и incremental capture читают `Content` одним chunked
механизмом с одинаковыми per-file/aggregate limits и checkpoint между chunks;
`Ignored` payload не входит в aggregate byte accounting.
