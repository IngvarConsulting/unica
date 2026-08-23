---
id: INV.APP.DAEMON-ACTOR-CAPACITY
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::daemon_actor_registry_is_bounded_weak_and_alias_safe
scope: [app, cache]
---

# Daemon удерживает только bounded множество живых WorkspaceActor

Daemon registry хранит weak entries и удаляет умершие перед admission. Один
canonical physical identity переиспользует живой actor; разные identity не
сливаются. Не более 64 одновременно живых actor capabilities допускаются,
активные actor не вытесняются, а последовательные завершённые workspaces не
увеличивают retained physical handles без границы.
