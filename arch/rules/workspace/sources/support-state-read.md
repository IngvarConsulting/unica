---
id: INV.APP.SUPPORT-STATE
check:
  - crates/unica-coder/src/infrastructure/support_state.rs::support_state_reader_rejects_edt_instead_of_claiming_not_supported
  - crates/unica-coder/src/infrastructure/support_state.rs::platform_xml_configuration_support_distinguishes_absent_unreadable_and_invalid_marker
  - crates/unica-coder/src/infrastructure/support_state.rs::platform_xml_support_reader_rejects_marker_below_linked_ext_directory
  - crates/unica-coder/src/infrastructure/support_state.rs::platform_xml_object_support_uses_the_resolved_descriptor_uuid
  - crates/unica-coder/src/infrastructure/application_ports.rs::meta_info_passes_its_resolved_target_to_support_reader
  - crates/unica-coder/src/infrastructure/application_ports.rs::meta_info_maps_support_provider_failure_to_logical_diagnostic
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_subsystem_view_preserves_support_for_own_and_nested_descriptors
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_view_distinguishes_absent_support_from_unreadable_and_invalid_markers
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::typed_projection_never_leaks_provider_or_physical_slots
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Невозможность прочитать поддержку не означает её отсутствие

Читатель состояния поддержки определяет конфигурацию или объект по логическому
адресу и выбирает реализацию для формата исходников. Если читать этот формат
он не умеет, возвращается ошибка, а не ответ «не на поддержке».

Для доказанной конфигурации Platform XML отсутствие `ParentConfigurations.bin`
означает отсутствие поддержки. Нечитаемый, повреждённый файл или ссылка вместо
него не дают такого ответа: чтение завершается ошибкой. Поддержка объекта
определяется по UUID его проверенного описания, а не по имени каталога.

Канонический `view` объекта или подсистемы показывает состояние поддержки
в `props` по логическому адресу.
