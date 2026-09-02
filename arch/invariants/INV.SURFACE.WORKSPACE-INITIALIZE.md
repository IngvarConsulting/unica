---
id: INV.SURFACE.WORKSPACE-INITIALIZE
status: active
governs: product
decision: DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS
check: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_previews_and_applies_workspace_initialization_before_admission
scope: [source, wire]
---

# Autodetected source sets инициализируют workspace только через fenced preview/apply

`workspace.initialize` доступен до source admission, требует явного `dryRun`,
не меняет файлы при preview и create-only публикует `v8project.yaml` только с
revision из того же плана. Реализованный source-only срез не требует платформы,
не перезаписывает config и не выбирает один формат для смешанных EDT/Designer
source sets.
