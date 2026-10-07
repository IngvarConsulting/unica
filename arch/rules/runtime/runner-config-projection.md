---
id: INV.RUNTIME.RUNNER-011-CONFIG-PROJECTION
check:
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::projection_preserves_overlay_paths_and_writes_private_files
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::target_config_is_projected_without_losing_origin_or_provider_meaning
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::unrepresentable_configuration_is_rejected_not_discarded
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::legacy_infobase_in_the_project_file_merges_with_the_local_infobases
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::infobase_without_a_local_layer_moves_into_a_private_local_layer
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::an_edt_cli_location_is_kept_relative_to_the_workspace
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::a_config_the_runner_reads_itself_is_passed_through_unchanged
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::mixing_spellings_across_layers_is_allowed
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::execution_timeout_is_refused_before_the_runner_starts_in_either_layer
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::partial_load_threshold_is_refused_before_the_runner_starts
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::the_projection_directory_is_stable_per_working_copy
  - crates/unica-coder/src/infrastructure/daemon/runner_013.rs::a_moved_cache_keys_the_projection_by_the_working_copy
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::a_refused_project_config_is_not_reported_as_an_absent_runner
---

# Адаптация конфигурации не меняет пользовательский проект

Служебная проекция в формат раннера 0.13 сохраняет цель `origin`, наложение
локального файла, смысл providers и исходную базу относительных путей.
Описание базы раннер читает только из локального слоя,
поэтому проекция сводит `origin` обоих файлов в приватную копию локального
слоя: поле локального файла перекрывает поле основного. Секреты локального
слоя не переносятся в основной слой. Исходные файлы проекта не
переписываются.

Приватная копия лежит в постоянном каталоге рабочей копии в кеше Unica:
`.build/unica/runner-project/`, а при кеше, вынесенном `UNICA_CACHE_DIR`, —
в его `runner-project/<ключ корня рабочей копии>/`. Каталог не входит
в исходники; на Unix он доступен только владельцу. Раннер записывает
владельцем файловой базы каталог прочитанного конфига, поэтому каталог один
на рабочую копию и переживает команду: так другая рабочая копия видит
владельца живым и получает отказ занятой базы. Каждая команда, которой нужна
проекция, перезаписывает копию (местный слой — только если он есть). Команда,
конфиг которой раннер читает сам, удаляет каталог, и владельцем становится
сама рабочая копия. Удаление каталога (вместе с рабочей копией или чисткой
кеша) делает владельца ушедшим: следующая команда записи другой копии берёт
базу. Обратного перехода адаптер не закрывает: если владельцем записан корень
рабочей копии, а конфиг потом потребовал проекции, раннер видит в корне другую
живую копию и отказывает `infobase_held`; запись корня снимается из метки
владельца вручную.

Непредставимая конфигурация отклоняется до запуска раннера: другая или
вторая база, смешение `infobase` и `infobases` в одном файле,
неподдерживаемый provider, `execution_timeout` и порог частичной загрузки
`partialLoadThreshold`, у которых в раннере нет смысла, не отбрасываются ради запуска с изменённым смыслом. Отказ называет
поле конфигурации, а не отсутствие раннера.
