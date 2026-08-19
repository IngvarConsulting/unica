---
id: INV.APP.PROVIDER-NEUTRAL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/code_intelligence.rs
scope: [app]
---

# Анализ кода и диагностики не зависят от движка

Поставщики кодовой интеллектуальности и диагностик подключаются через нейтральный порт,
сохраняют происхождение наблюдения и отказывают независимо друг от друга.
