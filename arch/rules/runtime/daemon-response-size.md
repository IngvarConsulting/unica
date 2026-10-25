---
id: INV.WIRE.DAEMON-RESPONSE-SIZE
check:
  - crates/unica-daemon/src/client_v5.rs::malformed_v5_response_permanently_poisons_owner_session
  - crates/unica-daemon/src/client_v5.rs::oversized_or_cut_v5_response_permanently_poisons_owner_session
  - crates/unica-coder/src/interfaces/daemon_router.rs::submit_answer_at_the_frame_limit_is_read_and_one_byte_over_or_cut_is_recovered
---

# Frontend самостоятельно ограничивает размер ответа демона

Ответ JSONL не превышает 8 МиБ + 64 КиБ. Предел ответа не выводится
из предела запроса: frontend проверяет его при чтении. Превышение,
некорректный или оборванный ответ закрывают соединение; отправлять следующий запрос
в это соединение нельзя.

Ответ ровно на пределе читается. На ответ на отправку вызова, который
превышает предел или оборван, frontend не отправляет вызов повторно,
а восстанавливает ту же квитанцию по ключу в новом соединении.
