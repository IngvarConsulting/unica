---
id: INV.WIRE.INFOBASE-CREATE-ONLY-CREATES-AN-ABSENT-ONE
status: superseded
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_infobase_create_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::an_existing_infobase_is_refused_at_preview_and_points_to_import
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_creates_and_takes_its_receipt_from_a_repeated_preview
scope: [wire, product]
---

# Создание базы не выдаёт чужую или существующую базу за созданную

`infobase.create` принимает только план со статусом `planned` у шага базы:
существующая база — отказ на превью с указанием на `infobase.import`, база,
появившаяся между превью и применением, — `concurrent_change`. Успех
подтверждается повторным превью раннера, где создавать больше нечего, и
называется засвидетельствованным провайдером; путь к базе и к платформе
наружу не идут.
