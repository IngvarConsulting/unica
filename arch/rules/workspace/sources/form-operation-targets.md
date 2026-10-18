---
id: INV.SOURCE.FORM-OPERATION-TARGETS
check:
  - crates/unica-coder/src/domain/apply.rs::a_form_item_role_narrows_element_operations
  - crates/unica-coder/src/domain/operation_contract.rs::form_move_contract_selects_one_source_and_preserves_exact_names
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::honest_can_on_form_items_and_events
  - crates/unica-coder/src/infrastructure/daemon/form_move_tests.rs::column_group_and_its_columns_are_addressable_through_the_wire
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::form_add_then_element_and_event_edits_compose_in_one_batch
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::a_form_edit_writes_only_the_addressed_item
---

# Операции формы учитывают роль элемента и его контейнер

Элемент и событие формы адресуются полной цепочкой `Form`, вложенных `Item`
и `Event`, которую публикует `view`. Автоматическая командная панель формы
или таблицы, группа колонок `ColumnGroup` и её колонки также видны как `Item`.
Редактор меняет только адресованный элемент; существование элемента
проверяется, отсутствие даёт `not_found` без записи.

`item.add` применим к форме или контейнеру, но не к полю, декорации или
кнопке. `item.remove` и `item.move` не удаляют и не переносят автоматическую
командную панель отдельно от владельца. `item.move` называет исходный
элемент его полным текущим адресом; обязательное `values.into` задаёт
контейнер строкой или корень формы явным `null`. `after` — имя соседа в том
же контейнере, чужой сосед даёт `bad_value`.

Реквизит и команду формы создают на адресе формы. `event.bind` на адресе
события принимает `values.handler` и `values.callType` и допускает каждое
событие, опубликованное узлом `Event` по каталогу платформы.
Общий порядок [выбора цели и отказа по снятым аргументам](../../mcp/operation-target-address.md)
применяется с этими условиями роли, без подмены операцией членов метаданных.
