---
id: INV.APP.RETAINED-APPLY-SUPPORT-POLICY-EVIDENCE
status: active
governs: product
decision: DEC.2026-08-26.RETAINED-APPLY-SUPPORT-POLICY-EVIDENCE-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_support_policy_evidence_contract_is_complete
scope: [app, platform, source]
---

# Retained apply проверяет exact support-policy evidence до результата

Actor admission удерживает bounded V12 candidate order, включая fixed-name
отсутствия и policy выше worktree. Planner видит только `Deny`, `Warn` или
`Off`; `Warn` и `Off` выводятся только из retained regular-file identity с
exact bytes. Evidence revalidation предшествует publication, завершает dry run
и входит в late final gate с rollback, не добавляя writer participant.
