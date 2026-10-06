---
id: INV.RUNTIME.COMPATIBLE-DEVELOPMENT-CYCLE
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::compatibility_cycle_accepts_explicit_force_and_rejects_silent_overwrite
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::compatibility_cycle_accepts_explicit_force_and_rejects_silent_overwrite
gap: https://github.com/IngvarConsulting/unica/issues/1246
---

# Цикл разработки сохраняет раздельные эффекты и явные ограничения

`push` исходников и `pull` требуют `force:true`. Отдельный режим
`push {delete: ...}` удаляет расширение по [плану расширения](extension-plan.md).
`push` исходников применяет конфигурацию БД; `noApply:true`
не поддержан. pull полностью заменяет один набор исходников. upload
принимает только квитанцию загрузки CF/CFE без обновления конфигурации БД.
apply/reset обращаются к основной конфигурации либо ровно одному расширению;
`reset` требует `force:true`. Плановые операции требуют явный boolean
`dryRun`: `true` показывает план, `false` исполняет текущий запрос.
Предварительный preview для исполнения не нужен; `ifRev` не принимается,
`rev` не выдаётся.
Проверка поколения базы не обещается. Изменения исполняет раннер.
