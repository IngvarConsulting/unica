---
id: INV.WIRE.SOURCE-IMPORT-APPLIES-THE-PREVIEWED-MODES
status: active
governs: product
decision: DEC.2026-09-15.SOURCE-IMPORT-FENCES-THE-PLAN-BY-ITS-MODES
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_source_import_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::preview_plans_every_declared_source_set_with_its_mode_without_dispatching_designer
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::apply_refuses_a_stale_revision_and_a_plan_that_changed_underneath
scope: [wire, product]
---

# Импорт исходников исполняет только одобренный план по составу и режимам

Превью `source.import` называет каждый объявленный набор с режимом, который
выбрал раннер, и не запускает конфигуратор. Применение принимается только по
`ifRev` этого превью и только если исполненный план совпал с одобренным по
составу наборов и режимам шагов; расхождение — `concurrent_change`. Состояние
базы после импорта названо засвидетельствованным провайдером; путь к
платформе наружу не идёт.
