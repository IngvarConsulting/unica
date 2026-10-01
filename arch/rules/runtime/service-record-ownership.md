---
id: INV.RUNTIME.SERVICE-RECORD-OWNERSHIP
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_cleanup_preserves_replacement_record
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_record_cleanup_serializes_concurrent_replacement
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_last_access_update_cannot_overwrite_replacement
---

# Старый сервис не меняет запись своего преемника

Завершение сервиса удаляет только принадлежащую ему запись. Если её уже
заменил новый процесс, прежний владелец не удаляет и не перезаписывает её
при обновлении времени последнего обращения. Это сохраняется и при
одновременных записи, обновлении и очистке.

Проверки управляют порядком настоящих файловых операций и блокировок.
