---
id: INV.APP.WORKSPACE-ACTOR-CAPABILITIES
status: active
governs: product
decision: DEC.2026-08-23.WORKSPACE-ACTOR-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_capabilities_reject_cross_instance_and_physical_rebinding
scope: [app, platform, source]
---

# Binding и fence принадлежат экземпляру и физическому корню актора

Provider binding и revision fence несут закрытую identity экземпляра актора и
не принимаются другим экземпляром с тем же структурным ключом. Актор удерживает
no-follow capability канонического source root: descriptor-relative чтение не
следует вложенной ссылке, а path-based результат и публикационная аренда
отклоняются, если имя корня стало обозначать другой физический каталог. Один
канонический или физический root нельзя объявить под двумя именами актора.
