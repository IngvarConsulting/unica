---
id: INV.CONFIG.INVALID-LAYER
check:
  - crates/unica-coder/src/infrastructure/operational_config.rs::invalid_shared_layer_is_not_masked_by_a_local_override
  - crates/unica-coder/src/infrastructure/operational_config.rs::invalid_local_overlay_is_not_ignored
  - crates/unica-coder/src/infrastructure/workspace_config.rs::rejects_version_as_unknown_root_field
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::characterization_policy_ignores_invalid_operational_subtree
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::characterization_operational_loader_ignores_invalid_policy_subtrees
---

# Корректное переопределение не скрывает ошибку файла настроек

Присутствующие файлы проверяются до слияния значений. Ошибка общего слоя
не исправляется корректным локальным значением; ошибочный локальный слой
не пропускается ради общего.

Синтаксис TOML и корневые поля `operational`, `network`, `providers` общие
для потребителей; неизвестное поле, в том числе `version`, отклоняется.
Внутри секций каждый потребитель проверяет свою область: ошибка сетевой
политики не мешает загрузить операционные настройки, и наоборот.
