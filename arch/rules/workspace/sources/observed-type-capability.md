---
id: INV.SOURCE.OBSERVED-TYPE-CAPABILITY
check:
  - crates/unica-coder/src/domain/metadata/observed_types.rs::read_only_observation_cannot_be_narrowed_into_the_writer_algebra
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_localizes_an_unknown_but_valid_platform_type_as_a_warning
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_keeps_a_broken_qualifier_in_the_error_severity_branch
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::uuid_writer_round_trips_through_meta_edit_and_info
---

# Распознанный тип не обязательно доступен для изменения

Внутренняя модель чтения метаданных различает распознанный тип и возможность
его записи. Тип с возможностью `readOnly` нельзя передать в модель записи.
UUID доступен для записи и возвращается чтением как UUID.

Незнакомый, но синтаксически допустимый платформенный тип даёт предупреждение
у соответствующего элемента; остальные сведения сохраняются. Повреждённый
квалификатор типа остаётся ошибкой, а не превращается в предупреждение
о неподдерживаемой возможности.

Проверки относятся к модели типов и прежнему внутреннему маршруту чтения
и изменения метаданных; они не доказывают все представления `unica.view`.
