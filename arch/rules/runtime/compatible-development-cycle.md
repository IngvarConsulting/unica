---
id: INV.RUNTIME.COMPATIBLE-DEVELOPMENT-CYCLE
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::force_is_optional_and_only_force_overwrites_the_infobase
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::compatibility_cycle_accepts_explicit_force_and_rejects_silent_overwrite
gap: https://github.com/IngvarConsulting/unica/issues/1246
---

# Цикл разработки сохраняет раздельные эффекты и явные ограничения

`pull` требует `force:true`. `push` исходников без `force` идёт со сверкой
поколения базы раннером, `force:true` перезаписывает базу без неё. Отдельный режим
`push {delete: ...}` удаляет расширение по [плану расширения](extension-plan.md).
`push` исходников применяет конфигурацию БД; `noApply:true`
не поддержан. pull полностью заменяет один набор исходников. upload
принимает только квитанцию загрузки CF/CFE без обновления конфигурации БД.
apply/reset обращаются к основной конфигурации либо ровно одному расширению;
`reset` требует `force:true`. Плановые операции требуют явный boolean
`dryRun`: `true` показывает план, `false` исполняет текущий запрос.
Предварительный preview для исполнения не нужен; `ifRev` не принимается,
`rev` не выдаётся.
Изменения исполняет раннер; сверку поколения базы перед загрузкой набора
ведёт он же, её границы описывает [план импорта исходников](source-import-plan.md).
