---
id: INV.WIRE.SURFACE-RELEASE-ROUTING
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/interfaces/mcp.rs::surface_release_structurally_gates_v12_legacy_dispatch_from_v13_daemon_dispatch
scope: [app, wire]
---

# V12 и hidden V13 имеют взаимоисключающие dispatch paths

Package-selected V12 вызывает только legacy handler. Явно injected V13 вызывает
только canonical daemon handler и не может fallback на legacy execution.
Публичный package остаётся V12 до атомарного cutover.
