---
id: INV.APP.DOCUMENTATION-NETWORK-POLICY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/documentation_policy.rs::default_deny_denies_only_providers_without_their_own_allow
scope: [app]
---

# Сетевая политика по умолчанию допускает только явно разрешённого поставщика

При общем запрете сети поставщик с собственным разрешением доступен, а другой
сетевой поставщик получает `policy-denied`; локальный поставщик не затронут.
