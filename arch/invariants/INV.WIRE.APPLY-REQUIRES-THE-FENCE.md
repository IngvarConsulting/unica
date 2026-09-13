---
id: INV.WIRE.APPLY-REQUIRES-THE-FENCE
status: active
governs: product
decision: DEC.2026-09-10.APPLY-FENCE-IS-A-CONTRACT
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::two_plans_on_one_revision_cannot_both_publish
  - crates/unica-coder/src/application/v13/apply.rs::the_fence_is_required_by_the_mode_and_the_schema_says_so
scope: [wire, product]
---

# Применение без забора ревизии не публикуется

`unica.apply` с `dryRun: false` без `ifRev` отказывает до записи. Два плана,
построенных на одной ревизии, не публикуются оба: опоздавший назван
устаревшим, а правка успевшего цела.

Условие объявлено в опубликованной схеме, а не только в описании поля: при
`dryRun: false` и при опущенном `dryRun` схема требует `ifRev`.
