---
id: INV.RUNTIME.SERVICE-CONTROL-CAPACITY
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_admission_ninth_work_runs_with_exact_cancel_ping_and_shutdown
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_admission_sixty_fifth_partial_header_reaches_handler_and_executes_work
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_admission_ninth_control_delivers_exact_cancel_with_all_old_slots_held
  - crates/unica-coder/src/infrastructure/workspace_services.rs::slow_general_headers_cannot_exhaust_control_capacity
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_concurrent_work_preserves_control_path
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Занятые обработчики не блокируют приём и управление сервисом

Число рабочих, общих или управляющих обработчиков само по себе не отклоняет
новую работу, проверку доступности, отмену или остановку внутреннего сервиса.
Пределы 8/64/8 и вытеснение из 64 ожидающих соединений не применяются.
Для принятого соединения сервис пытается создать обработчик без численной
квоты; работа исполняется отдельно от обработчика управления. Незавершённые заголовки других соединений не
закрывают доставку отмены точной операции.

Проверки токена и уникальности идентификатора операции сохраняются.
Явная остановка отменяет зарегистрированную работу и закрывает новый приём
по [правилу остановки сервиса](service-shutdown.md). Реальный отказ ОС создать
поток отличается от искусственной квоты.

TCP-проверки удерживают реальные обработчики и операции за прежними
границами; предметный исполнитель управляется тестом. Отдельный fallback
классификатора не нужен после снятия общей квоты, поэтому его отсечки
500 мс и 64 КиБ также отсутствуют.

Сроки заголовка, запроса и жизни сервиса, ограничения основных кадров,
отмена при EOF и освобождение после shutdown grace остаются в `gap`.
Их пересмотр и сохранность фактического владения продолжаются в #1119;
снятие численных квот не доказывает завершение этих путей.
