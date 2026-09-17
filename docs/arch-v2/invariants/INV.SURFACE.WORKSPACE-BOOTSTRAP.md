---
id: INV.SURFACE.WORKSPACE-BOOTSTRAP
status: active
governs: product
decision: DEC.2026-09-03.INFOBASE-EXPORT-RUN-SLICE
check: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_bootstraps_an_empty_workspace_before_address_discovery
scope: [source, wire]
---

# Пустое рабочее пространство наблюдаемо до admission исходников

`unica.view {}` успешно возвращает missing-состояние, не создавая файлов и не
требуя source set. Пустой workspace сообщает одну первичную проблему без
нерелевантного health и без рецепта на несуществующий source root. Autodetected
однородные source sets получают `workspace.initialize` preview; смешанные форматы не
сворачиваются в ложный глобальный рецепт, существующий config не предлагается
перезаписать, а health не расходует response serialization margin.

Configured workspace только с `infobase.connection` является готовым
runtime-состоянием, а не ошибкой исходников: он не получает `sourceSetExample`
или `source_roots_missing`, но получает точные preview-продолжения для CF и DT.
