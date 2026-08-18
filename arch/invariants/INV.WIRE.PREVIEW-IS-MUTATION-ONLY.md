---
id: INV.WIRE.PREVIEW-IS-MUTATION-ONLY
status: active
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs
scope: [wire]
---

# Предпросмотр принадлежит мутации

Читатель не принимает режим предпросмотра: у чтения нет последствий, которые стоило бы
показывать заранее.
