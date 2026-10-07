---
id: INV.WIRE.SOURCE-IMPORT-APPLIES-THE-PREVIEWED-MODES
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::preview_plans_every_declared_source_set_with_its_mode_without_dispatching_designer
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::preview_of_one_source_set_with_full_rebuild_asks_the_runner_for_exactly_that
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::preview_refuses_other_sets_a_dispatched_designer_and_edt_sources
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::direct_apply_checks_its_plan_and_attributes_the_infobase_state_to_the_provider
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::apply_refuses_a_plan_that_changed_during_execution
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::apply_where_the_runner_skipped_every_set_names_the_infobase_state_unverified
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::captured_runner_skip_is_answered_as_an_unverified_infobase_state
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::apply_names_only_loaded_sets_as_changed_and_the_skipped_ones_as_unverified
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::apply_refuses_a_dispatch_flag_that_contradicts_the_steps
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::apply_that_skips_a_previewed_load_is_a_concurrent_change
gap: https://github.com/IngvarConsulting/unica/issues/950
---

# Импорт исходников подтверждает состав и режимы своего плана

Отправка исходников через `push` требует `force:true` и применяет конфигурацию БД.
`noApply:true` не поддерживается; контроля поколений базы нет.

Preview `push` не запускает конфигуратор и называет каждый
выбранный набор исходников с режимом `full`, `partial` или `skipped`. Без `sourceSet`
план охватывает все объявленные наборы; `full` требует полного
режима. План для другого состава или EDT отклоняется.

Для `push` обязателен явный boolean `dryRun`: `true` возвращает preview,
`false` исполняет операцию без предварительного запроса preview.
Исполненный план сравнивается с планом текущего вызова по составу наборов
и режимам; несовпадение даёт `concurrent_change`. Состояние базы после импорта называется
засвидетельствованным провайдером, путь платформы не публикуется.

Режим `skipped` раннер выбирает по своей памяти о прошлых загрузках
в рабочем каталоге, а не по базе. Пропущенный набор не загружался
и не применялся, поэтому ответ не называет его изменённым и не выдаёт
его состояние в базе за засвидетельствованное. Если пропущены все наборы,
ответ сообщает: загрузка не выполнялась, раннер не нашёл изменений
относительно своей памяти, состояние базы не проверено. Ответ называет
условие: полная загрузка нужна, если базу могли изменить вне этой рабочей
копии; если базу загружает только она, действий не требуется. Preview полной
загрузки предлагается с этим условием и не первым шагом превью. Ответ раннера, в котором признак запуска платформы
противоречит шагам, отвергается. Режим `partial` тоже выбирается по памяти
раннера; сверка с состоянием базы не обещается.

`run` не принимает `ifRev` и не выдаёт `rev`. Исполнение использует
текущие входы; отдельный preview не закрепляет проект или байты исходников.
Внутри вызова проектный файл, его локальное дополнение и объявленный
состав наборов сверяются до и после внутреннего preview раннера.
Их изменение даёт `concurrent_change` до исполнения.
Проверки используют управляемые ответы раннера; отказ после
его исполнения не означает откат изменений информационной базы.
