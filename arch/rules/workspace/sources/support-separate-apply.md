---
id: INV.SOURCE.SUPPORT-SEPARATE-APPLY
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::support_and_ordinary_edits_require_separate_batches_under_every_policy
gap: https://github.com/IngvarConsulting/unica/issues/942
---

# Поддержка меняется отдельным применением

Операции изменения поддержки не смешиваются с обычными правками в одном
`apply`, даже если человек согласовал обе части. Если правка требует снятия
запрета, сначала отдельно применяется изменение поддержки, затем — правка
объекта. Отказ этой правки не отменяет уже применённого изменения поддержки.

Это ограничение состава запроса действует независимо от `deny`, `warn`
и `off`; сама [политика реакции на запрет](../support-policy.md) сохраняется.
