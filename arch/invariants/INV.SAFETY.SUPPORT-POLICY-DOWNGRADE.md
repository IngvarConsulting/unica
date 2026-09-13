---
id: INV.SAFETY.SUPPORT-POLICY-DOWNGRADE
status: active
governs: product
decision: DEC.2026-08-21.PROJECT-SUPPORT-POLICY
check: crates/unica-coder/src/infrastructure/support_guard.rs::project_editing_policy_is_the_closed_support_guard_downgrade_source
scope: [app, product]
---

# Ослабление блокировки поддержки имеет один источник

Только `editingAllowedCheck=warn|off` подходящего `.v8-project.json` ослабляет
реакцию на обнаруженную блокировку; отсутствие, повреждение и прочие значения
сохраняют запрет.
