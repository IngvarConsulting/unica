---
id: INV.SOURCE.LOGICAL-QUESTION-COMPLETENESS
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/filter_criterion_content.rs::filter_criterion_declarations_keep_dangling_targets_and_follow_real_nested_targets
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_reader_rejects_an_extra_unconsumed_address_tail
  - crates/unica-coder/src/infrastructure/daemon/server.rs::a_retired_read_filter_routes_next_to_the_address_that_replaced_it
---

# Чтение не игнорирует лишнюю часть адреса или неизвестный аргумент

Проектор должен разрешить весь логический адрес. Если после найденного
узла остаётся неразрешённая часть, ответ — `not_found`, а не данные
найденного родителя. Неизвестный аргумент чтения, в том числе снятый
`filter`, возвращает `bad_value`; его нельзя молча проигнорировать.

Проверки лишнего продолжения проходят XDTO, командный интерфейс и тело
метода, а снятого `filter` — `view` и `diff`. Они не заменяют проверку
каждого нового читателя.
