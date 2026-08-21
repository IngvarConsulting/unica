---
id: INV.APP.DOCUMENTATION-NETWORK-POLICY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/documentation_policy.rs::default_deny_denies_only_providers_without_their_own_allow
scope: [app]
---

# Явное разрешение перекрывает общий сетевой запрет

При `network.default = deny` поставщик `v8std` с собственным `allow` получает
`NetworkAccess::Allow`, а `kb-1ci` без собственного правила —
`NetworkAccess::Deny`.
