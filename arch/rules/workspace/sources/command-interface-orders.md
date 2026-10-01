---
id: INV.SOURCE.COMMAND-INTERFACE-ORDERS
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::the_command_interface_shows_every_order_it_holds
---

# Чтение командного интерфейса сохраняет объявленный порядок

Для чтения командного интерфейса подсистемы предусмотрены ветви `Command`,
`Group` и `Subsystem`. Группы панели идут в порядке `GroupsOrder`, команды внутри
группы — в порядке `CommandsOrder`, подсистемы — в порядке `SubsystemsOrder`.
Группа без команд остаётся видимой. Ссылка на подсистему дополняется именем
набора исходников до логического адреса.

`Group` обозначает группу панели, а не объект метаданных `CommandGroup`.
Чтение не скрывает разобранную секцию порядка.
