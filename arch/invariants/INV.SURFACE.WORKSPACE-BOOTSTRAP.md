---
id: INV.SURFACE.WORKSPACE-BOOTSTRAP
status: active
governs: product
decision: DEC.2026-09-01.VIEW-WORKSPACE-BOOTSTRAP
check: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_bootstraps_an_empty_workspace_before_address_discovery
scope: [source, wire]
---

# Пустое рабочее пространство наблюдаемо до admission исходников

`unica.view {}` успешно возвращает missing-состояние и рецепт
`v8project.yaml`, не создавая файлов и не требуя source set. Продолжение
публикуется только для ready Platform XML source с валидным логическим адресом,
смешанные форматы не сворачиваются в ложный глобальный рецепт, существующий
config не предлагается перезаписать, а health не расходует response
serialization margin.
