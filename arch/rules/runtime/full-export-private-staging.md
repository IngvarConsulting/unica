---
id: INV.RUNTIME.FULL-EXPORT-PRIVATE-STAGING
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::full_export_publishes_configuration_and_extension_from_a_private_stage
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::windows_runner_project_layers_are_owner_only_before_secret_bytes_are_written
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::windows_applied_full_dump_rejects_effective_config_acl_mutation_before_runner_and_cleans_created_identity
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::private_root_swap_cannot_escape_secret_scrubbing_or_anchor_cleanup
---

# Подготовка полной выгрузки не раскрывает рабочую конфигурацию

Временное дерево и эффективная конфигурация полной выгрузки недоступны
посторонним пользователям с момента создания. В Windows объекты сразу
получают защищённый DACL владельца без наследования; последующее ужесточение
прав не заменяет эту гарантию. Если закрытость не подтверждена, раннер
не запускается.
