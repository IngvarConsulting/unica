---
id: CTR.APP.DAEMON-LONG-WORK-CAPABILITIES
status: active
governs: product
decision: DEC.2026-08-24.LONG-WORK-OWNERSHIP-SLICE
check: crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_exact_long_work_ownership_contract
scope: [app]
version: 1
producer: crates/unica-coder/src/infrastructure/daemon/server.rs
consumers: [review]
---

# Скрытые capability долгой работы daemon

`ActorBoundExecution` предоставляет injected canonical service три закрытых
capability: actor-owned Index join, daemon-owned rootless ProviderHost join и
daemon-owned RuntimeResource join. Каждый KnownLong вызов сначала возвращает
durable Task, затем ждёт producer; polling не запускает producer повторно.
Capability не являются MCP tools, не добавляют TaskStore phase и не обходят
ProviderRootBinding, revision fence, `BslIndexLock` или runtime `active.lock`.
Production subject handler и durable progress остаются границей Task 22.
