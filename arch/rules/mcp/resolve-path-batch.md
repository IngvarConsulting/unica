---
id: INV.WIRE.RESOLVE-PATH-BATCH
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_paths_translates_paths_and_lines_in_input_order
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_paths_places_lines_by_the_method_range_resolve_names
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_paths_refuses_only_the_failing_item
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_paths_cancelled_read_refuses_the_whole_call
  - crates/unica-coder/src/application/v13/resolve.rs::resolve_paths_items_split_a_trailing_line_and_refuse_only_themselves
---

# Resolve переводит пакет путей и строк в адреса одним вызовом

`unica.resolve` с `paths` принимает элементы `путь` или `путь:строка`, где
строка — физическая строка файла с единицы. Ответ содержит по элементу
на каждую позицию входа в том же порядке, включая повторы. Путь без строки
даёт адрес и вид предмета с теми же доказательствами, что одиночный `path`
по [правилу точечного разрешения](../workspace/sources/source-path-resolution.md).

Строка файла BSL даёт метод, чей
[собственный диапазон](resolved-source-location.md) её содержит; строка вне
всех методов — адрес модуля с явным признаком этого. Строка древовидного
файла даёт его предмет с признаком, что строка предметом не является.
Угадывать предмет по строке нельзя: ответ точный или отказ.

Отказ элемента — неразборчивый элемент, путь вне допущенных наборов,
незарегистрированный предмет, строка за концом файла — стоит на позиции
этого элемента со своим кодом и не отменяет ответы остальных. Отмена
и истёкший срок относятся ко всему вызову.
