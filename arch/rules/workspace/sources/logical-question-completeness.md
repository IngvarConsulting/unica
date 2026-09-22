---
id: INV.SOURCE.LOGICAL-QUESTION-COMPLETENESS
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_reader_rejects_an_extra_unconsumed_address_tail
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::unsupported_view_filter_is_a_typed_bad_value_instead_of_a_noop
---

# Чтение не игнорирует лишнюю часть адреса или неизвестный фильтр

Проектор должен разрешить весь логический адрес. Если после найденного
узла остаётся неразрешённая часть, ответ — `not_found`, а не данные
найденного родителя. Неизвестное поле фильтра возвращает `bad_value`;
его нельзя молча проигнорировать.

Проверки лишнего продолжения проходят XDTO, командный интерфейс и тело
метода, а неизвестного фильтра — конфигурацию. Они не заменяют проверку
каждого нового читателя.
