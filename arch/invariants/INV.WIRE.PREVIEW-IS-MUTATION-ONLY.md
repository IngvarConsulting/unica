---
id: INV.WIRE.PREVIEW-IS-MUTATION-ONLY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::reader_rejects_dry_run_before_workspace_discovery
scope: [wire]
---

# Предпросмотр принадлежит мутации

Читатель не принимает режим предпросмотра: у чтения нет последствий, которые стоило бы
показывать заранее.
