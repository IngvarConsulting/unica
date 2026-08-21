---
id: INV.SAFETY.PREVIEW-BY-DEFAULT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::mutating_tool_defaults_to_dry_run_and_reports_cache
scope: [app, product]
---

# Мутация без явного применения остаётся предпросмотром

Мутирующий инструмент без явного `dryRun: false` возвращает результат сухого
прогона и сообщает влияние на кеш, не выдавая команду для применения.
