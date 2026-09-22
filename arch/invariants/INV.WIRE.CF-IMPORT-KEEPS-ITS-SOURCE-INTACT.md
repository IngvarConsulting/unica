---
id: INV.WIRE.CF-IMPORT-KEEPS-ITS-SOURCE-INTACT
status: superseded
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_cf_import_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::preview_names_the_source_and_the_target_without_touching_the_infobase
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::apply_refuses_a_provider_that_touched_the_source_or_reported_nothing_applied
scope: [wire, product]
---

# Загрузка CF/CFE не трогает свой источник и не выдаёт состояние базы за проверенное

`upload` применяется только по `ifRev` своего превью; превью ничего не
применяет и называет тот же файл и ту же цель, что запрошены. После загрузки
входной файл обязан остаться байт в байт прежним — иначе ответ отказ, а не
успех. Состояние базы после загрузки Unica не проверяет и в ответе называет
его засвидетельствованным провайдером; пути платформы и журнала наружу не
идут.
