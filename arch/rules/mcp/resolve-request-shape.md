---
id: INV.WIRE.RESOLVE-EXACT-BRIDGE
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/application/v13/resolve.rs::resolve_takes_exactly_one_side_of_the_bridge
  - crates/unica-coder/src/application/v13/resolve.rs::absent_lines_are_named_rather_than_omitted
gap: https://github.com/IngvarConsulting/unica/issues/976
---

# Запрос resolve выбирает одну сторону адресного моста

`unica.resolve` принимает ровно один аргумент: логический адрес `at`
или путь `path`. Два аргумента вместе и отсутствие обоих дают `bad_value`.

Форма расположения явно называет наличие строк: `lines.state` равен
`range` с границами `from` и `to` либо `notLineBased`. Поле не опускается,
когда диапазона нет. Эти проверки закрепляют разбор запроса и сериализацию
расположения. Они не доказывают существование указанного узла и правильность
его диапазона в исходниках.

При успешном ответе указанный предмет существует; результат сообщает его
адрес, вид и действительное место в исходниках. Отсутствующий предмет даёт
`not_found`. Для метода или области BSL диапазон относится именно к файлу
модуля, а не к XML-дескриптору его владельца. У древовидного ресурса диапазон
не выдумывается. Эти гарантии текущего вызова требуют проверки из `gap`.
