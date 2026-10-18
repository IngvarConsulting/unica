---
id: INV.RUNTIME.RUNNER-011-CONFIG-PROJECTION
check:
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::projection_preserves_overlay_paths_and_writes_private_files
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::target_config_is_projected_without_losing_origin_or_provider_meaning
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::unrepresentable_configuration_is_rejected_not_discarded
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::legacy_infobase_in_the_project_file_merges_with_the_local_infobases
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::infobase_without_a_local_layer_moves_into_a_private_local_layer
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::an_edt_cli_location_is_kept_relative_to_the_workspace
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::a_config_the_runner_reads_itself_is_passed_through_unchanged
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::mixing_spellings_across_layers_is_allowed
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::execution_timeout_is_refused_before_the_runner_starts_in_either_layer
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::partial_load_threshold_is_refused_before_the_runner_starts
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::the_projection_directory_is_stable_per_working_copy
  - crates/unica-coder/src/infrastructure/daemon/runner_015.rs::a_moved_cache_keys_the_projection_by_the_working_copy
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::a_refused_project_config_is_not_reported_as_an_absent_runner
---

# Адаптация конфигурации не меняет пользовательский проект

Служебная проекция в формат раннера сохраняет цель `origin`, наложение
локального файла, смысл providers и исходную базу относительных путей.
Описание базы сводится в приватную копию локального слоя: поле локального
файла перекрывает поле основного. Секреты локального слоя не переносятся
в основной; исходные файлы проекта не переписываются.

Приватная копия лежит вне исходников в постоянном каталоге рабочей копии
в кеше Unica: `.build/unica/runner-project/`, а при `UNICA_CACHE_DIR` —
в `runner-project/<ключ корня рабочей копии>/` этого кеша. На Unix каталог
доступен только владельцу. Стабильный каталог сохраняет владение файловой
базой между командами и предупреждение при записи другой рабочей копией.
Проекция обновляется по текущим слоям без оставшегося лишнего локального
файла; команда без проекции освобождает её каталог. Удаление каталога
делает прежнего владельца ушедшим, и другая копия может взять базу без
предупреждения. Обратный переход от корня рабочей копии к проекции
адаптер не закрывает: раннер предупреждает о другом живом владельце
и не меняет метку. Ручное восстановление описано в
[справке проекта](../../../plugins/unica/references/tooling/v8project.md#file-infobase-holder).

Непредставимая конфигурация отклоняется до запуска раннера. Другая или
вторая база, смешение `infobase` и `infobases` в одном файле,
неподдерживаемый provider, `execution_timeout` и `partialLoadThreshold`
не отбрасываются ради запуска с изменённым смыслом. Отказ называет поле
конфигурации, а не отсутствие раннера.
