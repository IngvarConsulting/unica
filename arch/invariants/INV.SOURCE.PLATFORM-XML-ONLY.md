---
id: INV.SOURCE.PLATFORM-XML-ONLY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/tool_context.rs::native_platform_xml_source_format_guard_is_closed_over_public_operations
scope: [source]
---

# Нативная граница не принимает EDT

Каждая зарегистрированная `NativeOperation` применяет одинаковый гейт формата:
принимает явный `platform_xml` и исторический `unknown`, но отклоняет EDT и
недопустимую или неоднозначную классификацию типизированной ошибкой.
