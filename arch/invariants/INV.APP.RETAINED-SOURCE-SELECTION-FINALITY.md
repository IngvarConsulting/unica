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
границ. `v8project.yaml` и успешно содержательно классифицируемый
`ConfigDumpInfo.xml` удерживаются по exact bytes; oversized descriptor может
оставить recoverable `format_probe_error` только вместе с typed terminal
observation retained parent/name/identity и `length > maximum`; existence-only
marker удерживает retained parent,
имя и `FileIdentity`. Поэтому изменение exact content, absence, wrong-kind,
membership, terminal oversized class или physical identity любой строки
исключает result/receipt, а in-place bytes existence-only marker не образуют
ложную семантическую зависимость. Actor probe без terminal observation закрывает
admission.

Каждый проход дедуплицирует одинаковые наблюдения, отвергает противоречивые и
применяет общие, не per-source-set, retained-state пределы: 32 MiB canonical
exact bytes, 65 536 evidence records, 128 retained directories и 8 MiB
route/name bytes. Отдельные pass-global work пределы — 32 MiB суммы metadata
length каждого exact observation и 16 384 перечисленных членов. Перечисленные
члены включают повторы, отрицательные не XML и wrong-kind inputs. Exact repeat
списывает work до streaming compare с canonical bytes без второго полного
буфера. Membership учитывает применимые record/name/member capacity до
enumeration, directory capacity — до возможного child open, а exact reader —
следующий chunk до расширения. Regular handle закрывается после capture, первый
snapshot не держит handles, второй держит не более 128 directory capabilities.
Сравнение заимствованного evidence проверяет deadline/cancellation в ходе работы.
Поздний отказ восстанавливает существующими двумя writers source, cache и revision
state; отдельного writer или durable source-selection recipe нет.
