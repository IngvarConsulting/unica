---
id: INV.APP.DEFERRED-READ
status: active
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: crates/unica-coder/src/application/result_store.rs::opaque_view_cursor_retry_is_idempotent_and_bound_to_the_complete_question
scope: [app]
---

# Продолжение читает неизменяемый сохранённый снимок

Повтор одного курсора возвращает ту же страницу, не переспрашивая источник:
курсор связан с полным вопросом и ревизией, а смена ревизии делает его
устаревшим вместо тихого ответа из другого снимка.
