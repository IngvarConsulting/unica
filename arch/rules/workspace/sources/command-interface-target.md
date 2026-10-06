---
id: INV.SOURCE.COMMAND-INTERFACE-TARGET
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_interface_edits_reach_exactly_the_nested_owner_and_refuse_foreign_targets
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_interface_order_permutes_stored_composition_and_refuses_any_other_without_writing
---

# Настройка командного интерфейса выбирает его владельца

`commandVisibility.set`, `commandPlacement.set`, `commandOrder.set`
и `groupOrder.set` выбирают интерфейс подсистемы по адресу
`…Subsystem.<Имя>.Interface`. Команда указывается в аргументах.
`subsystemOrder.set` выбирает корень `Configuration`.

Вложенная подсистема сохраняет полную цепочку владельцев;
одноимённая подсистема в другом месте не становится целью.
Отдельного корневого маршрута `main:Interface` нет.
