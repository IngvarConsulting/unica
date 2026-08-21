---
id: INV.SAFETY.PREVIEW-BY-DEFAULT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::every_mutator_defaults_to_preview_without_touching_storage
scope: [app, product]
---

# Мутация без явного применения остаётся предпросмотром

Каждый мутирующий инструмент без явного `dryRun: false` проходит свою
объявленную стратегию предпросмотра, сообщает влияние на кеш и не меняет байты
workspace, состояние, индекс и записи скрытых сервисов.
