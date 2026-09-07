---
id: INV.APP.RETAINED-APPLY-CLOSED-PARTICIPANTS
status: active
governs: product
decision: DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::retained_transaction_roles_require_explicit_roots_and_cache_authority
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::arbitrary_second_transaction_cannot_masquerade_as_actor_cache_authority
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::closed_transaction_rejects_physical_alias_and_second_cache_participant
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_admission_rejects_source_inside_cache
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_root_source_allows_exact_generated_cache_descendant
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_root_source_and_missing_cache_publish_through_disjoint_shared_anchor
scope: [app, cache, platform, source]
---

# Retained apply принимает ровно Source и WorkspaceCache одного actor

Одна actor-issued writer authority связывает по одному явному retained root
ролей `Source` и `WorkspaceCache`, включая no-op plan. Cache participant обязан
совпасть с отдельно выданной actor cache authority; arbitrary second
transaction, повторная роль, foreign authority и совпадение logical roots
отклоняются. Общий retained ancestor допустим только для exact `.build/unica`
внутри workspace-root source при непересекающихся target paths; Source role не
может адресовать ни один component `.build`. Source внутри cache и физический
alias самостоятельных roots fail closed.
