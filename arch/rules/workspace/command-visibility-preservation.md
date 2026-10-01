---
id: INV.WIRE.COMMAND-INTERFACE-WRITE
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::command_visibility_edits_common_and_leaves_role_values_alone
---

# Общая видимость команды сохраняет значения по ролям

`commandVisibility.set` меняет общее значение видимости команды
в `CommandInterface.xml` и сохраняет её ролевые переопределения.
