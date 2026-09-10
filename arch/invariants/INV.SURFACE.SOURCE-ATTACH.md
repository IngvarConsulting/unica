---
id: INV.SURFACE.SOURCE-ATTACH
status: superseded
governs: product
decision: DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS
check: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_hands_the_project_file_recipe_without_an_initialize_operation
scope: [source, wire]
---

# Autodetected source sets присоединяются только через fenced preview/apply

`source.attach` доступен до source admission, требует явного `dryRun`, не меняет
файлы при preview и create-only публикует `v8project.yaml` только с revision из
того же плана. Он не требует платформы, не перезаписывает config и не выбирает
один формат для смешанных EDT/Designer source sets.
