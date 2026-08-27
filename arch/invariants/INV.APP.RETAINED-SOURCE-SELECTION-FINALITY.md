---
id: INV.APP.RETAINED-SOURCE-SELECTION-FINALITY
status: active
governs: product
decision: DEC.2026-08-27.RETAINED-SOURCE-SELECTION-EVIDENCE-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_source_selection_finality_contract_is_complete
scope: [app, source, platform, cache]
---

# Apply публикуется только при неизменном retained выборе источников

Actor и apply admission получают полную карту из двух совпавших retained
descriptor-relative проходов. Apply переносит невызываемое и несериализуемое
evidence всех входов карты до prepublication, dry-result и retained-final
границ; изменение exact, absence, wrong-kind, membership или physical identity
любой строки исключает result/receipt. Поздний отказ восстанавливает существующими
двумя writers source, cache и revision state; отдельного writer или durable
source-selection recipe нет.
