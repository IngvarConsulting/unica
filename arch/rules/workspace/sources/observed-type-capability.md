---
id: INV.SOURCE.OBSERVED-TYPE-CAPABILITY
check:
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::canonical_uuid_attribute_round_trips_as_an_editable_observed_type
  - crates/unica-metadata/src/observed_types.rs::read_only_observation_cannot_be_narrowed_into_the_writer_algebra
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_localizes_an_unknown_but_valid_platform_type_as_a_warning
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_localizes_an_unmodelled_constant_type_with_the_warning_code
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_keeps_a_broken_qualifier_in_the_error_severity_branch
  - crates/unica-coder/src/infrastructure/native_operations/meta/info_projection_tests.rs::observed_type_reads_an_extension_type_id_as_a_read_only_variant
---

# Распознанный тип не обязательно доступен для изменения

Внутренняя модель чтения метаданных различает распознанный тип и возможность
его записи. Тип с возможностью `readOnly` нельзя передать в модель записи.
UUID доступен для записи и возвращается чтением как UUID.

Незнакомый, но синтаксически допустимый платформенный тип даёт предупреждение
у соответствующего элемента; остальные сведения сохраняются. Повреждённый
квалификатор типа остаётся ошибкой, а не превращается в предупреждение
о неподдерживаемой возможности.

Запись и чтение UUID проверяют канонические `apply` и `view`; предупреждение
и ошибка типа пока доказаны только моделью типов и внутренним маршрутом
чтения (#1933). Все представления `unica.view` проверки не доказывают.
