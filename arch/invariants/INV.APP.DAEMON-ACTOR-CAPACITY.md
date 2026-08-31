---
id: INV.APP.DAEMON-ACTOR-CAPACITY
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_workspace_actor_admission_is_concurrent_bounded_and_fail_closed
scope: [app, cache]
---

# Daemon удерживает только bounded множество живых WorkspaceActor

Daemon registry хранит weak entries и удаляет умершие перед admission. Один
canonical physical identity переиспользует живой actor; разные identity не
сливаются. Не более 64 одновременно живых actor capabilities допускаются,
активные actor не вытесняются, а последовательные завершённые workspaces не
увеличивают retained physical handles без границы. Одновременные admission
одной identity создают один actor. Отказ по capacity не увеличивает registry и
возвращает retryable `workspace_capacity`; poisoned registry не маскируется под
ошибку caller и возвращает закрытый `workspace_registry_failed`.
