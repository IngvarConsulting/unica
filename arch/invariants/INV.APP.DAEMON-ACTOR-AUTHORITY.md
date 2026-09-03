---
id: INV.APP.DAEMON-ACTOR-AUTHORITY
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/daemon/mod.rs::canonical_invocation_authority_is_actor_bound_and_revision_fenced
scope: [app, cache]
---

# Canonical Invocation исполняется только с authority точного WorkspaceActor

После дешёвой schema-проверки daemon связывает canonical call с opaque
`ActorBoundInvocation`: retained exact actor, named physical provider root и
identity digest, полученный от того же actor. Handler не получает raw
`InvocationRequest` или `workspaceHint`. Чтение и terminal publication проходят
через actor root validation и source revision fence; замена root или revision
отклоняет staged result без раскрытия его байтов.
