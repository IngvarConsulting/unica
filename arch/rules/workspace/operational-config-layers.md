---
id: INV.CONFIG.OPERATIONAL-LAYERS
check:
  - crates/unica-coder/src/infrastructure/operational_config.rs::local_config_overrides_fields_and_inherits_the_rest
  - crates/unica-coder/src/infrastructure/operational_config.rs::local_config_is_valid_without_shared_config
  - crates/unica-coder/src/infrastructure/operational_config.rs::a_changed_file_is_observed_by_the_next_load_only
  - crates/unica-coder/src/infrastructure/operational_config.rs::separate_workspaces_do_not_share_a_process_global_snapshot
---

# Операционные настройки загружаются отдельно для рабочего пространства

Загрузчик берёт настройки из `unica.toml` и `unica.local.toml` в корне
рабочего пространства. Локальное значение перекрывает общее по отдельному
полю; для остальных полей сохраняются общее значение или умолчание.
Локальный файл допустим без общего.

Новая загрузка замечает изменение файла. Ранее полученный снимок не меняется,
а настройки одного рабочего пространства не подменяют настройки другого.
