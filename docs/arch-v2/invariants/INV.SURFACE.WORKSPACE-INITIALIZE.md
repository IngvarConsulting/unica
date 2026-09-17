---
id: INV.SURFACE.WORKSPACE-INITIALIZE
status: superseded
governs: product
decision: DEC.2026-09-03.INFOBASE-EXPORT-RUN-SLICE
check: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_hands_the_project_file_recipe_without_an_initialize_operation
scope: [source, wire]
---

# Autodetected source sets инициализируют workspace только через fenced preview/apply

**Снято.** `workspace.initialize` ушёл со словаря `run`
(`DEC.2026-09-09.PROJECT-CONFIG-IS-HANDWRITTEN`). Правило остаётся историей, а
его проверка указывает на доказательство снятия: содержимое проектного файла
отдаётся в `setup`, а операции на проводе нет.

`workspace.initialize` доступен до source admission, требует явного `dryRun`,
не меняет файлы при preview и create-only публикует `v8project.yaml` только с
revision из того же плана. Реализованный source-only срез не требует платформы,
не перезаписывает config и не выбирает один формат для смешанных EDT/Designer
source sets.
