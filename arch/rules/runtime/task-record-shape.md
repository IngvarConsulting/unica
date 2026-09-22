---
id: INV.APP.DAEMON-TASK-PERSISTENCE
check:
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
