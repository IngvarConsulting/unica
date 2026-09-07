---
id: INV.APP.DEFERRED-MANIFEST
status: active
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: crates/unica-coder/src/application/result_store.rs::cursor_chain_is_refused_before_it_can_exceed_the_entry_bound
scope: [app]
---

# Большое чтение отвечает ограниченной страницей

Ответ сверх порога не выдаётся целиком: `view` отдаёт страницу и
непрозрачный курсор, связанный с вопросом и ревизией. Цепочка курсоров
ограничена, а чужой или устаревший курсор отказывается вместо чужого ответа.
