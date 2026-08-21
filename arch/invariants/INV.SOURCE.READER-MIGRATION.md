---
id: INV.SOURCE.READER-MIGRATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::bridged_readers_publish_two_mutually_exclusive_selector_branches
scope: [source]
---

# Режим миграции читателя объявлен явно

Перечень предметных читателей в режиме `bridge` сохраняет для каждого из них
взаимоисключающие логический и файловый входы до отдельного снятия. Ни один из
перечисленных читателей не выдаётся за уже завершивший прямой переход.
