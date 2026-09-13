---
id: INV.APP.META-FINDINGS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/meta_add_surface_tests.rs::info_uses_nonempty_list_presentation_for_command_text_finding
scope: [app]
---

# Находка метаданных несёт код, поле и язык

Предупреждение о длине представления списка публикует стабильный код,
`properties.ListPresentation` и фактический язык `ru`.
