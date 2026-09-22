---
id: INV.SOURCE.LOGICAL-READER-PARITY
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_rejects_direct_typed_owner_absent_from_configuration_inventory
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_rejects_orphan_nested_module_owners_not_registered_by_parent
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::registered_physical_child_with_wrong_descriptor_fails_direct_and_parent_navigation
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::registered_top_level_owner_without_descriptor_fails_kind_branch_and_direct_view
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::orphan_and_missing_physical_children_fail_closed_across_reader_families
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::external_parent_childobjects_are_the_only_nested_owner_authority
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::unregistered_top_level_descriptors_cannot_enter_any_typed_reader_family
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::object_commands_are_registered_inline_without_descriptor_files
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::add_in_templates_stop_addressing_at_the_template_without_reading_the_payload
---

# Файл становится логическим объектом только через регистрацию владельца

Для чтения объекта `view` проверяет его регистрацию у владельца.
Отдельно лежащий дескриптор, форма, макет или модуль команды не создаёт
объект в логическом дереве: обращение к нему получает `not_found`.
Это относится и к вложенным объектам внешней обработки.

Зарегистрированные объекты, формы и макеты требуют соответствующего
дескриптора. Отсутствие или неверный вид дескриптора даёт
`provider_unavailable` как при прямом обращении, так и при чтении
содержащей его коллекции или владельца.

Команда регистрируется в `ChildObjects` владельца и не требует отдельного
XML-дескриптора. Её встроенное описание может задавать отображаемое имя.

Зарегистрированный макет AddIn с корректным дескриптором доступен как узел
макета. Для этого Unica не разбирает его бинарное содержимое и не объявляет
внутренних ветвей.
