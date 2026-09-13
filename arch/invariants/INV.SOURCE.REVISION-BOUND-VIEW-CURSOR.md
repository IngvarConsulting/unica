---
id: INV.SOURCE.REVISION-BOUND-VIEW-CURSOR
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
check:
  - crates/unica-coder/src/application/result_store.rs::opaque_view_cursor_retry_is_idempotent_and_bound_to_the_complete_question
  - crates/unica-coder/src/application/result_store.rs::exact_revision_change_is_stale_but_tampering_and_expiry_are_invalid
  - crates/unica-coder/src/application/result_store.rs::cursor_chain_is_refused_before_it_can_exceed_the_entry_bound
scope: [app, product, source]
---

# Cursor логического view непрозрачен, ограничен и привязан к exact revision

Cursor хранит целую уже прочитанную страницу и стабильный successor в
ограниченном process-local store. Он связан с canonical address, выбранной
проекцией, normalized filter, source-set identity, exact source revision и
page limit. Повтор того же cursor-а в TTL возвращает ту же страницу и тот же
successor.

Изменившаяся revision даёт `stale_cursor`. Unknown, tampered, expired или
cross-address/projection/filter/source-set/limit cursor даёт `invalid_cursor`.
Нарезка, которая превысила бы entry bound, не материализует cursor chain.
