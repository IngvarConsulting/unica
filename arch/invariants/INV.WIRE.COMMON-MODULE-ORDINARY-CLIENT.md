---
id: INV.WIRE.COMMON-MODULE-ORDINARY-CLIENT
status: active
governs: product
decision: DEC.2026-08-22.COMMON-MODULE-ORDINARY-CLIENT
check: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_apply_view_round_trip_common_module_ordinary_client
scope: [wire]
---

# Apply и view сохраняют свойство обычного клиента

`props.set` в `unica.apply` записывает boolean `ClientOrdinaryApplication`
для `CommonModule`; `unica.view` читает `true` и `false` в `props.commonModule.clientOrdinaryApplication`.
Preview сохраняет файлы и наблюдаемую ревизию. Неверный тип значения и попытка
записать свойство документа отклоняются с `bad_value` в preview и apply,
сохраняя файлы и ревизию.
