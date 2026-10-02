---
id: INV.RUNTIME.COMPATIBLE-DEVELOPMENT-CYCLE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_development_cycle_prepares_before_source_admission_and_keeps_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::compatibility_cycle_accepts_explicit_force_and_rejects_silent_overwrite
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::compatibility_cycle_accepts_explicit_force_and_rejects_silent_overwrite
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::upload_refuses_a_receipt_that_implicitly_applied_the_database
  - crates/unica-coder/src/infrastructure/daemon/v13_configuration_transition.rs::transitions_apply_only_the_previewed_target_and_never_lose_the_force_flag
---

# Цикл разработки сохраняет раздельные эффекты и явные ограничения

`push` исходников и `pull` требуют `force:true`. Отдельный режим
`push {delete: ...}` удаляет расширение по [плану расширения](extension-plan.md).
`push` исходников применяет конфигурацию БД; `noApply:true`
не поддержан. pull полностью заменяет один набор исходников. upload
принимает только квитанцию загрузки CF/CFE без обновления конфигурации БД.
apply/reset обращаются к основной конфигурации либо ровно одному расширению;
`reset` требует `force:true`. Исполнение требует `ifRev` своего превью.
Проверка поколения базы не обещается. Изменения исполняет раннер.
