---
id: INV.SOURCE.EXACT-ROOT-FORM
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_owner.rs::existing_form_content_resolves_exact_wrapper_and_source_set_owners
scope: [source]
---

# Содержимое формы проверяет точный корень оболочки

Существующая объявленная цель управляемой формы разрешает владельцев только
через точный корень своей оболочки и окружающего набора исходников до записи.
