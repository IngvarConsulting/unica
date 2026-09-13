---
id: INV.SAFETY.SUPPORT-GUARD-COVERAGE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/application/mod.rs::mutating_native_support_guard_coverage_is_explicit
  - crates/unica-coder/src/application/mod.rs::code_patch_locked_support_blocks_preview_and_apply_before_handler
  - crates/unica-coder/src/application/mod.rs::subsystem_compile_guards_locked_parent_before_both_planners
  - crates/unica-coder/src/application/mod.rs::cf_init_support_exemption_reaches_preview_and_apply_handlers
  - crates/unica-coder/src/infrastructure/support_guard.rs::project_editing_policy_is_the_closed_support_guard_downgrade_source
scope: [app, product]
---

# Каждая нативная мутация явно классифицирована по защите поддержки

Дескриптор каждой нативной мутации либо требует редактируемого владельца, либо
входит в закрытый перечень операций, не меняющих объект на поддержке.
