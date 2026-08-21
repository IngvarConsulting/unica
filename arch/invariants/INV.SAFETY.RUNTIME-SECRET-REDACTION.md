---
id: INV.SAFETY.RUNTIME-SECRET-REDACTION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/internal_adapters.rs::production_secret_redaction_surfaces_are_closed
scope: [app, product]
---

# Задание runtime не публикует секреты

Терминальный снимок, сохранённая запись, аргументы и хвосты журналов задания
runtime скрывают секретные значения, сохраняя пути к файловым журналам.
