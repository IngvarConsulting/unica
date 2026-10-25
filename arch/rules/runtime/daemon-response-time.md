---
id: INV.WIRE.DAEMON-RESPONSE-TIME
check:
  - crates/unica-daemon/src/client_v5.rs::partial_v5_response_bytes_cannot_replenish_client_deadline
  - crates/unica-coder/tests/platform/receipt_runtime.rs::submit_response_deadline_is_its_budget_plus_one_margin_under_the_safety_cap
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_trickling_submit_answer_does_not_renew_the_frontend_deadline
  - crates/unica-coder/tests/platform/receipt_runtime.rs::fail_stop_transport_margin_starts_after_response_serialization
gap: https://github.com/IngvarConsulting/unica/issues/984
---

# Время передачи ответа не продлевает исполнение операции

Для обычного submit к бюджету прямого ответа вызова добавляется 125 мс на сериализацию
и передачу ответа. Частичное поступление байтов не начинает срок заново.

Аварийная доставка закрытой ошибки fail-stop имеет отдельный запас. Этот запас
не разрешает продолжить предметную операцию.

Исходный срок фиксируется один раз, до строгой проверки запроса, допуска
и подготовки операции. Он определяет момент прямого ответа или передачи
вызова в задание; работа операции срока не получает и после этого момента
продолжается в задании. Все этапы до ответа используют этот срок и тот же
источник времени. Совпадение числовой отметки другого источника времени не делает
её исходным сроком вызова.
