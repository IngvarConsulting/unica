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
  - crates/unica-coder/src/infrastructure/v13_read/tests/form_support.rs::canonical_form_support_keeps_own_rule_and_owner_prohibition
  - crates/unica-coder/src/infrastructure/v13_read/tests/form_support.rs::canonical_form_support_unknown_is_not_safe_and_absence_has_a_reason
  - crates/unica-coder/src/infrastructure/v13_read/tests/form_support.rs::canonical_form_support_common_form_has_no_invented_owner_in_configuration_or_extension
  - crates/unica-coder/src/infrastructure/v13_read/tests/form_support.rs::canonical_form_support_refuses_invalid_marker_and_missing_uuid_even_when_globally_locked
  - crates/unica-coder/src/infrastructure/v13_read/tests/form_support.rs::canonical_form_support_facts_do_not_depend_on_editing_policy
  - crates/unica-coder/src/infrastructure/v13_read/tests/form_support.rs::canonical_form_support_unknown_owner_does_not_turn_editable_child_into_safe
  - crates/unica-coder/src/infrastructure/v13_read/tests/form_support.rs::canonical_form_support_refuses_linked_marker_and_ext
  - crates/unica-coder/src/infrastructure/v13_read/tests/form_support.rs::canonical_form_support_details_do_not_change_legacy_payload
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
