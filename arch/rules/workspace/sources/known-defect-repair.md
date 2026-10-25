---
id: INV.SOURCE.KNOWN-DEFECT-REPAIR
check:
  - crates/unica-coder/src/infrastructure/daemon/server/legacy_inline_check_tests.rs::an_owner_edit_registers_legacy_inline_forms_and_templates_by_name
  - crates/unica-coder/src/infrastructure/daemon/server/legacy_inline_check_tests.rs::an_edit_outside_the_owner_description_keeps_legacy_inline_registrations
  - crates/unica-coder/src/infrastructure/daemon/server/legacy_inline_check_tests.rs::canonical_check_warns_about_legacy_inline_form_and_template_without_writes
---

# Ремонт известной поломки

Если apply затрагивает узел с поломкой, которую Unica умеет исправить,
предпросмотр называет поломку и исправление, а исполнение по токену плана
его выполняет. check называет поломку и правку, которая её чинит.
Нетронутые узлы не чинятся.

Например, встроенное во владельца описание формы или макета рядом
с отдельным описанием той же формы или макета (#1370).
