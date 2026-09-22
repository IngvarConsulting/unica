---
id: INV.APP.DAEMON-TASK-PERSISTENCE
check:
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::create_exact_uses_preallocated_identity_and_is_idempotent_only_for_exact_readback
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::post_publish_sync_failure_is_commit_uncertain_with_exact_visible_readback
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::open_inspect_only_preserves_nonterminal_bytes_and_catalogues_exact_state
  - crates/unica-coder/src/application/invocation_store_v5.rs::all_five_task_statuses_round_trip_with_the_exact_selected_fields
  - crates/unica-coder/src/application/invocation_store_v5.rs::record_rejects_wrong_schema_unknown_duplicate_and_every_missing_root_field
  - crates/unica-coder/src/application/invocation_store_v5.rs::every_task_status_rejects_unknown_missing_wrong_case_and_cross_variant_fields
  - crates/unica-coder/src/application/invocation_store_v5.rs::v5_safe_failure_reason_is_closed_and_converts_every_legacy_reason
gap: https://github.com/IngvarConsulting/unica/issues/984
---

# Сохранённая задача имеет закрытую форму для каждого состояния

Запись задачи протокола v5 содержит идентичность вызова, хеши аргументов
и рабочего пространства, сроки и состояние. Декодер отклоняет неизвестную
версию схемы, лишние, повторяющиеся и отсутствующие обязательные поля.

Допустимы пять состояний: `queued`, `working`, `completed`, `failed`
и `cancelled`. Результат хранится только в `completed`, причина ошибки —
только в `failed`. Поля другого состояния отклоняются.

Причина ошибки выбирается из закрытого перечня `V5SafeFailureReason`.
Произвольный текст вместо причины не принимается.

Повторное открытие хранилища сохраняет номер версии записи и признак
отмены; оно само не переписывает незавершённые записи. Последующее
восстановление заданий описано в [отдельном правиле](task-recovery.md).

Полная сохранённая запись не превышает 8 МиБ + 64 КиБ. Проверка предельного
сохранения и отказа без изменения прежней записи остаётся в `gap`.

Новая запись использует заранее выделенный `taskId`. Повтор её создания
разрешён только для точно совпавшей идентичности вызова. Другой вызов
под тем же идентификатором отклоняется без замены записи.

Если запись уже опубликована, а синхронизация с диском не подтверждена,
она остаётся учтённой и доступной для точной сверки. Неопределённый исход
синхронизации не означает, что записи нет.
