---
id: INV.RUNTIME.RUNNER-011-CONFIG-PROJECTION
check:
  - crates/unica-coder/src/infrastructure/daemon/runner_011.rs::projection_preserves_overlay_paths_and_removes_private_files_on_failure
  - crates/unica-coder/src/infrastructure/daemon/runner_011.rs::target_config_is_projected_without_losing_origin_or_provider_meaning
  - crates/unica-coder/src/infrastructure/daemon/runner_011.rs::unrepresentable_configuration_is_rejected_not_discarded
---

# Адаптация конфигурации не меняет пользовательский проект

Служебная проекция в формат раннера 0.11 сохраняет цель `origin`, наложение
локального файла, смысл providers и исходную базу относительных путей.
Секреты локального слоя не переносятся в основной слой. Исходные файлы
проекта не переписываются, приватные копии удаляются и при ошибке.

Непредставимая конфигурация отклоняется: другая или вторая база, смешение
старого и нового описания цели, неподдерживаемый provider не отбрасываются
ради запуска с изменённым смыслом.
