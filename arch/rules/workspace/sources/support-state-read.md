---
id: INV.APP.SUPPORT-STATE
check:
  - crates/unica-coder/src/infrastructure/support_state.rs::support_state_reader_rejects_edt_instead_of_claiming_not_supported
  - crates/unica-coder/src/infrastructure/support_state.rs::platform_xml_configuration_support_distinguishes_absent_unreadable_and_invalid_marker
  - crates/unica-coder/src/infrastructure/support_state.rs::platform_xml_support_reader_rejects_marker_below_linked_ext_directory
  - crates/unica-coder/src/infrastructure/support_state.rs::platform_xml_object_support_uses_the_resolved_descriptor_uuid
  - crates/unica-coder/src/infrastructure/application_ports.rs::meta_info_passes_its_resolved_target_to_support_reader
  - crates/unica-coder/src/infrastructure/application_ports.rs::meta_info_maps_support_provider_failure_to_logical_diagnostic
gap: https://github.com/IngvarConsulting/unica/issues/986
---

# Невозможность прочитать поддержку не означает её отсутствие

Читатель состояния поддержки определяет конфигурацию или объект по логическому
адресу и выбирает реализацию для формата исходников. Если читать этот формат
он не умеет, возвращается ошибка, а не ответ «не на поддержке».

Для доказанной конфигурации Platform XML отсутствие `ParentConfigurations.bin`
означает отсутствие поддержки. Нечитаемый, повреждённый файл или ссылка вместо
него не дают такого ответа: чтение завершается ошибкой. Поддержка объекта
определяется по UUID его проверенного описания, а не по имени каталога.

Проверки охватывают внутренний читатель поддержки и чтение метаданных через
порт приложения. Они не доказывают все представления публичного `unica.view`.

Канонический `view` объекта или подсистемы показывает состояние поддержки
в `props` по логическому адресу. Вызывающий не должен искать физический файл
состояния поддержки. В проекции подсистемы этот факт пока теряется; разрыв
и недостающая сквозная проверка сохранены в `gap`.
