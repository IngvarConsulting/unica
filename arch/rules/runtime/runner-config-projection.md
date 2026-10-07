---
id: INV.RUNTIME.RUNNER-011-CONFIG-PROJECTION
check:
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::projection_preserves_overlay_paths_and_removes_private_files_on_failure
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::target_config_is_projected_without_losing_origin_or_provider_meaning
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::unrepresentable_configuration_is_rejected_not_discarded
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::legacy_infobase_in_the_project_file_merges_with_the_local_infobases
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::infobase_without_a_local_layer_moves_into_a_private_local_layer
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::an_edt_cli_location_is_kept_relative_to_the_workspace
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::a_config_the_runner_reads_itself_is_passed_through_unchanged
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::mixing_spellings_across_layers_is_allowed
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::execution_timeout_is_refused_before_the_runner_starts_in_either_layer
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::partial_load_threshold_is_refused_before_the_runner_starts
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::a_refused_project_config_is_not_reported_as_an_absent_runner
---

# Адаптация конфигурации не меняет пользовательский проект

Служебная проекция в формат раннера 0.13 сохраняет цель `origin`, наложение
локального файла, смысл providers и исходную базу относительных путей.
Описание базы раннер читает только из локального слоя,
поэтому проекция сводит `origin` обоих файлов в приватную копию локального
слоя: поле локального файла перекрывает поле основного. Секреты локального
слоя не переносятся в основной слой. Исходные файлы проекта не
переписываются, приватные копии удаляются и при ошибке.

Непредставимая конфигурация отклоняется до запуска раннера: другая или
вторая база, смешение `infobase` и `infobases` в одном файле,
неподдерживаемый provider, `execution_timeout` и порог частичной загрузки
`partialLoadThreshold`, у которых в раннере нет смысла, не отбрасываются ради запуска с изменённым смыслом. Отказ называет
поле конфигурации, а не отсутствие раннера.
