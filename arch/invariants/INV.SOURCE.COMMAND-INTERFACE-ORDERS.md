---
id: INV.SOURCE.COMMAND-INTERFACE-ORDERS
status: active
governs: product
decision: DEC.2026-09-09.COMMAND-INTERFACE-ORDERS-VISIBLE
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::the_command_interface_shows_every_order_it_holds
scope: [product, source]
---

# Что интерфейс хранит, то он и показывает

Узел командного интерфейса подсистемы объявляет ветви `Command`, `Group` и
`Subsystem`; внутри группы лежат её команды в порядке `CommandsOrder`. Секция,
которую разбирает читатель, обязана иметь выход наружу: разобранный и не
показанный факт неотличим от несуществующего, а писать его будет нельзя.

Группа, объявленная в порядке и не имеющая команд, остаётся видимой. Ссылка
платформы в порядке подсистем дополняется до адреса.

Вид узла `Group` называет группу панели, а не объект метаданных
`CommandGroup`.
