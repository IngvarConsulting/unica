---
id: INV.APP.PROVIDER-NEUTRAL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/code_intelligence.rs::registry_resolves_an_executable_provider_for_read_capabilities
scope: [app]
---

# Читатель кода выбирается по capability поставщика

Реестр кодовой интеллектуальности выбирает поставщика по требуемой возможности,
а результат сохраняет идентичность фактически вызванного поставщика.
