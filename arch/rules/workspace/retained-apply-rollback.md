---
id: INV.CACHE.RETAINED-APPLY-REVISION-ROLLBACK
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::real_effect_after_all_postimages_cancellation_rolls_back_exact_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::real_effect_after_all_postimages_deadline_rolls_back_exact_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_failures_restore_source_cache_and_revision_machine_exactly
gap: https://github.com/IngvarConsulting/unica/issues/971
---

# Неудачное применение правок откатывается целиком

При обрабатываемой ошибке применения подготовленных правок Unica возвращает
изменённые исходники, связанный кеш и сведения о ревизии в состояние до начала
операции. Ревизия — внутренний идентификатор состояния исходников Unica.

Например, первый файл уже записан, а при записи второго произошла ошибка.
Первый файл должен вернуться к прежнему содержимому. Вместе с ним должны
восстановиться кеш и сведения о ревизии, чтобы Unica не считала правки
применёнными.

Тест вызывает сбой в пяти точках применения и сравнивает файлы и состояние
в памяти с сохранёнными до операции. Дополнительные проверки вызывают отмену
и истечение срока после записи всех подготовленных файлов; в этой точке
исходники, кеш и состояние ревизии тоже восстанавливаются. Восстановление
после аварийного завершения процесса эти проверки не доказывают.

Откат удаляет пустые каталоги, созданные этой партией. Сценарий с новой
цепочкой каталогов и чужой подменой ещё требует проверки, указанной в `gap`.
