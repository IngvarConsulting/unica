---
id: INV.CACHE.GENERATION-CUTOVER
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_index.rs::builder_15_uses_a_new_generation_and_leaves_builder_14_untouched
scope: [cache]
---

# Builder 15 использует отдельное поколение индекса

Команда builder 15 получает каталог `rlm-bsl/index-v15` и не меняет данные
предыдущего поколения builder 14.
