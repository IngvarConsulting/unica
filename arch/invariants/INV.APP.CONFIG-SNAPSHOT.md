---
id: INV.APP.CONFIG-SNAPSHOT
status: active
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/operational_config.rs
scope: [app]
---

# Конфигурация вызова разрешается снимком

Операционная конфигурация читается один раз на вызов и дальше не меняется под ним: два
одновременных вызова не влияют друг на друга через файл настроек.
