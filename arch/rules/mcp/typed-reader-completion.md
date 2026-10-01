---
id: INV.APP.TYPED-READER-COMPLETION
check:
  - crates/unica-coder/src/application/mod.rs::successful_typed_reader_without_data_fails_closed
  - crates/unica-coder/src/application/mod.rs::successful_typed_reader_with_stdout_duplicate_fails_closed
  - crates/unica-coder/src/application/mod.rs::failed_typed_reader_may_omit_data
---

# Типизированное чтение не завершается пустым успехом

Финализатор внутреннего контракта `Read + Typed` отклоняет успешный
ответ обработчика без предметных данных. Текстовый дубль в `stdout`
также недопустим. При отказе обработчика данные могут отсутствовать.

Проверки относятся к внутреннему пути прежних обработчиков. Они
не доказывают форму или полноту ответа канонического `view`.
