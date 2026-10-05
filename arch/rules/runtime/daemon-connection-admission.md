---
id: INV.RUNTIME.DAEMON-CONNECTION-ADMISSION
gap: https://github.com/IngvarConsulting/unica/issues/1119
check:
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::ninth_connection_delivers_cancel_while_eight_real_handshakes_are_held
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::sixty_fifth_authenticated_owner_delivers_exact_cancellation
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::connection_ownership_survives_former_limit_and_checked_overflow
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::v5_duplicate_live_owner_lease_is_rejected
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::live_v5_owner_prevents_idle_listener_shutdown
---

# Соединения демона сохраняют владельцев без произвольной квоты приёма

Число других соединений или аутентифицированных владельцев само по себе
не отклоняет новое соединение. Занятые соединения не закрывают возможность
доставить отмену точного вызова. Пределы восемь handshake и 64 owner lease
не применяются.

До завершения аутентификации соединение удерживает учёт приёма. Перед его
освобождением устанавливается owner lease: между этими стадиями владение
соединением не исчезает. Idle-очистка должна наблюдать оба состояния согласованно.
Owner lease сохраняется до фактического завершения обработчика; один lease
не принадлежит двум живым соединениям. Проверки версии, идентичности и токена
аутентификации сохраняются.

Арифметическое переполнение счётчика остаётся ошибкой и не повреждает учёт.
Правило относится к числу соединений; автоматические сроки и доставка
публичной MCP-отмены до Task ID изменяются отдельно в
[#1119](https://github.com/IngvarConsulting/unica/issues/1119) и
[#929](https://github.com/IngvarConsulting/unica/issues/929).

Разрыв в #1119: listener пока читает пустоту owner-реестра перед счётчиком
handshake. При переходе между этими чтениями он может ошибочно увидеть оба
состояния пустыми. Снятие квот не исправляет эту прежнюю гонку; согласованное
наблюдение и причинный concurrency-тест выполняются отдельно.
