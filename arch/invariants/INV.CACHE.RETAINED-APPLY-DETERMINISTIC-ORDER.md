---
id: INV.CACHE.RETAINED-APPLY-DETERMINISTIC-ORDER
status: active
governs: product
decision: DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_deterministic_success_and_rollback_order_is_complete
scope: [cache, platform, source]
---

# Retained apply публикует и откатывает участников в фиксированном порядке

Transaction observer видит Source postimages первыми, eager cache metadata
следующими, revision record затем и `state.json` последним. После отказа у
state marker journal откатывает опубликованные file postimages в точном
обратном порядке.
