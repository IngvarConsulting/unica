---
id: INV.SOURCE.BORROWED-OBJECT-REFRESH
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/borrow.rs::canonical_borrow_preview_refresh_and_noop_keep_local_identity_and_events_honest
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_object_borrow_previews_publishes_and_rejects_changed_parent_revision
  - crates/unica-coder/src/infrastructure/native_operations/cfe_borrow_object.rs::catalog_refresh_updates_platform_control_properties_and_keeps_override
---

# Повторное заимствование обновляет свойства из родителя

При явном повторном заимствовании того же объекта из того же родителя Unica
обновляет перенесённые свойства по текущему состоянию родителя. Свойства,
перекрытые в расширении, сохраняются; предпросмотр показывает изменения.
Если обновлять нечего, операция завершается успешно без изменений.
Неизменённые файлы не отмечаются как созданные или обновлённые.

Постоянная синхронизация с родителем из этого правила не следует.
Смена родителя и изменение запрошенного набора перекрытий — другие случаи.
