---
id: INV.WIRE.SURFACE-RELEASE-ROUTING
status: active
governs: product
decision: DEC.2026-08-31.V0-13-SURFACE-FIRST-CUTOVER
check: crates/unica-coder/src/interfaces/mcp.rs::surface_release_structurally_gates_v12_legacy_dispatch_from_v13_daemon_dispatch
scope: [app, wire]
---

# Package-selected V13 не возвращается в legacy dispatch

Package-selected V13 вызывает только canonical daemon handler и не может
fallback на legacy execution. Legacy V12 остаётся только изолированным тестовым
seam для доказательства взаимоисключающих маршрутов.
