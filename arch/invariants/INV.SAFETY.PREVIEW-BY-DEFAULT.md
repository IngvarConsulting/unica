---
id: INV.SAFETY.PREVIEW-BY-DEFAULT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/application_ports.rs::public_preview_strategies_are_real_and_recursively_write_free
scope: [app, product]
---

# Мутация без явного применения остаётся предпросмотром

Каждый мутирующий инструмент без явного `dryRun: false` проходит свою
объявленную стратегию предпросмотра, сообщает влияние на кеш и не меняет байты
workspace, состояние, индекс и записи скрытых сервисов.
