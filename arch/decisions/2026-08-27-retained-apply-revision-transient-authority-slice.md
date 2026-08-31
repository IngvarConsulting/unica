---
id: DEC.2026-08-27.RETAINED-APPLY-REVISION-TRANSIENT-AUTHORITY-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/source_revision.rs::retained_apply_revision_transient_authority_preserves_projection_capture_bounds
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.RETAINED-APPLY-TRANSIENT-ENTRY-AUTHORITY]
design: docs/design/2026-08-27-retained-apply-revision-transient-authority-design.md
---

# Journal владеет временным допуском retained revision capture

**Решение.** Planning capture сохраняет полный счётчик перечисленных entries;
retained-apply projection применяет к нему итоговую топологию batch и до
публикации проверяет entry/depth bounds живого scanner.

Во время postpublication validation journal выдаёт один sealed borrowed batch,
связывающий retained root с точными parent, recovery name и single-link regular
file capabilities вытесненных preimages. Оба enumeration pass обеих capture
могут не учитывать только доказанные этим batch физические entries, ровно по
одному разу. Произвольные ignored entries продолжают расходовать лимит.

Authority не клонируется, не сериализуется и прекращает существовать до
изменения journal, rollback или cleanup. Rollback повторно проверяет single-link
identity до восстановления. Остаток после cleanup никогда не получает authority
при следующем admission или restart.

**Почему.** Final tree на точном entry limit должен воспроизводиться, пока
journal сохраняет rollback preimage, но неаутентифицированный запас или prefix
ослабил бы retained bound для чужих файлов.

**Цена.** Scanner и transaction journal связаны внутренним borrowed proof;
каждый новый вид временного entry потребует отдельного доказанного расширения,
а не нового имени или allowance.
