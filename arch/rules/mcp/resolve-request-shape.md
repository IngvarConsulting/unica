---
id: INV.WIRE.RESOLVE-EXACT-BRIDGE
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/application/v13/resolve.rs::resolve_takes_exactly_one_side_of_the_bridge
  - crates/unica-coder/src/application/v13/resolve.rs::absent_lines_are_named_rather_than_omitted
---

# Запрос resolve выбирает одну сторону адресного моста

`unica.resolve` принимает ровно один аргумент: логический адрес `at`
или путь `path`. Два аргумента вместе и отсутствие обоих дают `bad_value`.

Форма расположения явно называет наличие строк: `lines.state` равен
`range` с границами `from` и `to` либо `notLineBased`. Поле не опускается,
когда диапазона нет. Эти проверки закрепляют разбор запроса и сериализацию
расположения. Они не доказывают существование указанного узла и правильность
его диапазона в исходниках.
