---
id: INV.CACHE.WORKSPACE-ACTOR-STATE-SCOPE
status: active
governs: product
decision: DEC.2026-08-23.WORKSPACE-ACTOR-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::remapped_names_and_profiles_do_not_share_revision_index_or_coordination_state
scope: [app, cache]
---

# Generic actor разделяет состояние по полной структурной identity

Ограниченный domain-separated state scope включает канонический workspace,
упорядоченные пары имени и канонического root каждого source set и точный
provider profile. Канонические пути кодируются стабильными native bytes без
lossy Unicode conversion и без provider-cache case folding; невозможность
стабильного кодирования закрывает создание generic actor ошибкой. Разные scope
не делят persisted source revision, index, provider cache, coordination или
background state. Только v0.12 workspace-service compatibility adapter до Task
22 использует прежний явный namespace `LegacyPhysical`.
