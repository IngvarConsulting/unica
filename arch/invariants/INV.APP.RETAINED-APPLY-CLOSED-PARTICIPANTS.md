---
id: INV.APP.RETAINED-APPLY-CLOSED-PARTICIPANTS
status: active
governs: product
decision: DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_closed_participant_contract_is_complete
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
