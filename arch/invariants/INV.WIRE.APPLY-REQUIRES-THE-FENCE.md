---
id: INV.WIRE.APPLY-REQUIRES-THE-FENCE
status: active
governs: product
decision: DEC.2026-09-10.APPLY-FENCE-IS-A-CONTRACT
check: crates/unica-coder/src/infrastructure/daemon/server.rs::two_plans_on_one_revision_cannot_both_publish
scope: [wire, product]
---

# Применение без забора ревизии не публикуется

`unica.apply` с `dryRun: false` без `ifRev` отказывает до записи. Два плана,
построенных на одной ревизии, не публикуются оба: опоздавший назван
устаревшим, а правка успевшего цела.
