---
id: INV.SOURCE.COMMAND-INTERFACE-SECTION-ISOLATION
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_each_interface_operation_changes_only_its_own_section_and_items
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_command_order_of_one_group_keeps_other_groups_and_sections
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_interface_edits_reach_exactly_the_nested_owner_and_refuse_foreign_targets
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::command_visibility_edits_common_and_leaves_role_values_alone
---

# Настройка интерфейса сохраняет соседние секции и группы

Каждая из пяти операций настройки интерфейса меняет только свою секцию
и выбранные элементы. Например, новый порядок команд одной группы
сохраняет порядок команд другой группы.

`commandVisibility.set` меняет общее значение видимости выбранной команды
в `CommandInterface.xml`, сохраняя её ролевые переопределения.
Остальные ролевые значения видимости, не затронутые запросом, также сохраняются.
