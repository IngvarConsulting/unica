---
id: INV.SOURCE.COMMAND-INTERFACE-TARGET
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_interface_edits_reach_exactly_the_nested_owner_and_refuse_foreign_targets
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_interface_order_permutes_stored_composition_and_refuses_any_other_without_writing
  - crates/unica-coder/src/domain/apply.rs::retired_names_with_a_dot_route_to_escaped_addresses
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_command_order_refuses_an_undeclared_group_without_writing
---

# Настройка командного интерфейса выбирает его владельца

`commandVisibility.set`, `commandPlacement.set` и `groupOrder.set`
выбирают интерфейс подсистемы по адресу `…Subsystem.<Имя>.Interface`.
`commandOrder.set` выбирает группу этого интерфейса
`…Interface.Group.<Группа>`; имя группы с точкой (`CommandGroup.Цены`)
экранируется по грамматике адреса, а аргумент `group` отклоняется с адресом
группы в `next`. Группа должна быть объявлена в интерфейсе, как её видит
чтение; неизвестная группа отвечает `not_found` без записи. Команда
указывается в аргументах.
`subsystemOrder.set` выбирает корень `Configuration`.

Вложенная подсистема сохраняет полную цепочку владельцев;
одноимённая подсистема в другом месте не становится целью.
Отдельного корневого маршрута `main:Interface` нет.
