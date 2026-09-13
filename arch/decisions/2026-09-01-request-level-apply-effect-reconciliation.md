---
id: DEC.2026-09-01.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::request_level_reconciliation_drops_cancelled_effect_before_deduplication
supersedes: [DEC.2026-08-28.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION]
superseded-by: null
establishes: [INV.APP.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION]
design: docs/design/2026-08-28-request-level-apply-effect-reconciliation-design.md
---

# Request-level apply публикует только surviving effects

**Решение.** Один apply request индексирует исходные операции, изменяет единый
staged state через typed family runs и передаёт provisional effects единому
request-level finalizer. Finalizer сначала оставляет только candidates, чьи
path-bound subjects изменены в финальном postimage, затем выполняет stable
deduplication и только после этого формирует actor-facing effect receipt.

**Почему.** Это сохраняет atomic request semantics: промежуточный effect,
отменённый последующей операцией, не становится наблюдаемым cache или domain
event результатом, а global operation index сохраняется при cross-family
ошибке.
