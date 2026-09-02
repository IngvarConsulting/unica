---
id: INV.SURFACE.WORKSPACE-BOOTSTRAP
status: active
governs: product
decision: DEC.2026-09-02.RUN-INITIALIZATION-CONTRACT
check: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_bootstraps_an_empty_workspace_before_address_discovery
scope: [source, wire]
---

# Пустое рабочее пространство наблюдаемо до admission исходников

`unica.view {}` успешно возвращает missing-состояние, не создавая файлов и не
требуя source set. Пустой workspace сообщает одну первичную проблему без
нерелевантного health и без рецепта на несуществующий source root. Autodetected
однородные source sets получают `source.attach` preview; смешанные форматы не
сворачиваются в ложный глобальный рецепт, существующий config не предлагается
перезаписать, а health не расходует response serialization margin.
