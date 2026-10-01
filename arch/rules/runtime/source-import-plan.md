---
id: INV.WIRE.SOURCE-IMPORT-APPLIES-THE-PREVIEWED-MODES
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::preview_plans_every_declared_source_set_with_its_mode_without_dispatching_designer
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::preview_of_one_source_set_with_full_rebuild_asks_the_runner_for_exactly_that
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::preview_refuses_other_sets_a_dispatched_designer_and_edt_sources
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::apply_repeats_the_preview_and_attributes_the_infobase_state_to_the_provider
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::apply_refuses_a_stale_revision_and_a_plan_that_changed_underneath
gap: https://github.com/IngvarConsulting/unica/issues/950
---

# Импорт исходников подтверждает состав и режимы своего плана

Отправка исходников через `push` адаптера 0.11.2 требует `force:true` и применяет конфигурацию БД.
`noApply:true` не поддерживается; контроля поколений базы нет.

Preview `push` не запускает конфигуратор и называет каждый
выбранный набор исходников с режимом `full` или `partial`. Без `sourceSet`
план охватывает все объявленные наборы; `full` требует полного
режима. План для другого состава или EDT отклоняется.

Применение требует `ifRev` preview. Исполненный план сравнивается
с одобренным по составу наборов и режимам; несовпадение даёт
`concurrent_change`. Состояние базы после импорта называется
засвидетельствованным провайдером, путь платформы не публикуется.

Ревизия связывает проектный файл, объявленный состав наборов, аргументы,
версию раннера и шаги плана с их режимами. Если эти входы изменились
после preview, прежний `ifRev` отклоняется до исполняющего вызова раннера.

Ревизия плана не связывает байты всего дерева: правка, сохранившая режимы,
может пройти. Проверки используют управляемые ответы раннера; отказ после
его исполнения не означает откат изменений информационной базы.
