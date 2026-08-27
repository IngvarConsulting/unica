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
границ. `v8project.yaml` и содержательно классифицируемый `ConfigDumpInfo.xml`
удерживаются по exact bytes; existence-only marker удерживает retained parent,
имя и `FileIdentity`. Поэтому изменение exact content, absence, wrong-kind,
membership или physical identity любой строки исключает result/receipt, а
in-place bytes existence-only marker не образуют ложную семантическую зависимость.

Каждый проход дедуплицирует одинаковые наблюдения, отвергает противоречивые и
применяет общие, не per-source-set, пределы: 32 MiB exact bytes, 65 536 evidence
records, 16 384 перечисленных членов, 128 retained directories и 8 MiB route/name
bytes. Перечисленные члены включают отрицательные не XML и wrong-kind inputs.
Квота новой уникальной записи проверяется до открытия/аллокации, где возможно;
regular handle закрывается после capture, первый snapshot не держит handles,
второй держит не более 128 directory capabilities. Сравнение заимствованного
evidence не клонирует bytes и проверяет deadline/cancellation в ходе работы.
Поздний отказ восстанавливает существующими двумя writers source, cache и revision
state; отдельного writer или durable source-selection recipe нет.
