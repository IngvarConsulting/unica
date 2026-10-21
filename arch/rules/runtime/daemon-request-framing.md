---
id: INV.WIRE.DAEMON-REQUEST-FRAMING
check:
  - crates/unica-daemon/src/protocol_v5.rs::bounded_v5_reader_rejects_oversized_empty_and_unterminated_frames
  - crates/unica-daemon/tests/platform/listener.rs::handshake_keeps_its_small_bound_while_an_authenticated_frame_may_be_far_larger
  - crates/unica-daemon/src/client_v5/request_write.rs::a_frame_above_the_request_bound_is_refused_with_both_sizes_before_writing
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_runs_a_large_call_and_refuses_one_above_the_bound_unsent
  - crates/unica-daemon/tests/platform/listener.rs::a_frame_straddling_the_idle_session_poll_is_read_whole
  - crates/unica-daemon/tests/platform/listener.rs::a_session_frame_missing_its_deadline_closes_the_session
---

# Демон читает целый кадр в его границах

Запрос — непустая завершённая строка JSONL; размер считается вместе
с переводом строки. До проверки токена первая строка ограничена 16 КиБ,
строка аутентифицированной сессии — 64 МиБ. Пустой, незавершённый
или превышающий предел кадр отвергается. Предел сессии временный, без
подтверждённого основания ([#1309](https://github.com/IngvarConsulting/unica/issues/1309),
[#1312](https://github.com/IngvarConsulting/unica/issues/1312)); он не доказывает
защиту суммарного расхода памяти всех сессий.

Frontend отказывает до отправки кадра сверх предела: канонический ответ
содержит `provider_limit_exceeded`, исход `fixCall` и диагностику с пределом
и фактическим размером в байтах, без содержимого аргументов. Квитанция
не создаётся, вызов не исполняется.

Срок чтения кадра сессии — 7 с от первого байта. Поступление следующих
байтов его не продлевает. Кадр, начатый в конце ожидания простаивающей
сессии, сохраняет уже прочитанное начало. Если кадр не дочитан к своему
сроку, сессия закрывается: его хвост не разбирается как новый запрос.
