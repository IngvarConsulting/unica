---
id: INV.SURFACE.SOURCE-ATTACH
status: active
governs: product
decision: DEC.2026-09-02.RUN-INITIALIZATION-CONTRACT
check: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_previews_and_applies_autodetected_source_attachment_before_admission
scope: [source, wire]
---

# Autodetected source sets присоединяются только через fenced preview/apply

`source.attach` доступен до source admission, требует явного `dryRun`, не меняет
файлы при preview и create-only публикует `v8project.yaml` только с revision из
того же плана. Он не требует платформы, не перезаписывает config и не выбирает
один формат для смешанных EDT/Designer source sets.
