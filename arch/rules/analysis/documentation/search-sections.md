---
id: INV.APP.DOCUMENTATION-SECTIONS
check:
  - crates/unica-coder/src/application/documentation.rs::sections_follow_registry_order_and_carry_provenance
  - crates/unica-coder/src/application/documentation.rs::one_failed_provider_does_not_hide_the_other
  - crates/unica-coder/src/application/documentation.rs::all_providers_failed_is_an_error
  - crates/unica-coder/src/application/documentation.rs::empty_status_counts_as_success_not_just_ok
  - crates/unica-coder/src/application/documentation.rs::warnings_reach_the_public_result_next_to_an_ok_status
---

# Поиск справки сохраняет ответы и происхождение каждого источника

Секции идут в порядке регистрации поставщиков. Секция называет вид источника,
авторитетность, корпус и фактический язык; попадание — версию документа.
Язык запроса не подменяет язык материала. Оценки разных секций не сравниваются,
а найденные документы не объединяются в общий рейтинг.

Сбой одного поставщика не скрывает ответы остальных. Поиск успешен, если
хотя бы один поставщик ответил, в том числе пустой выдачей. Когда пригодного
ответа нет ни у одного, вызов завершается отказом.

Предупреждение о неполноте корпуса сохраняется рядом с успешной выдачей.
Результат собирается после завершения запросов к поставщикам.
