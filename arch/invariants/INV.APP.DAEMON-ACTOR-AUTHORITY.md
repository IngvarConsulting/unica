---
id: INV.APP.DAEMON-ACTOR-AUTHORITY
status: active
governs: product
decision: DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::bound_task_start_rejects_missing_foreign_stale_actor_proof_without_mutation
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::bound_task_start_rechecks_proof_after_working_readback
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5.rs::authenticated_pre_cancel_submit_and_recover_cross_the_actor_owned_runtime
scope: [app, cache]
---

# Canonical Invocation исполняется только с authority точного WorkspaceActor

После дешёвой schema-проверки daemon связывает canonical call с opaque
`ActorBoundInvocation`: retained exact actor, named physical provider root и
identity digest, полученный от того же actor. Handler не получает raw
`InvocationRequest` или `workspaceHint`. Чтение и terminal publication проходят
через actor root validation и source revision fence; замена root или revision
отклоняет staged result без раскрытия его байтов.
