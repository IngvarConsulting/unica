---
id: INV.SOURCE.PLATFORM-XML-ONLY
status: active
governs: product
decision: DEC.2026-08-21.LEGACY-UNKNOWN-NATIVE-SOURCE-FORMAT
check: crates/unica-coder/src/infrastructure/tool_context.rs::native_platform_xml_source_format_public_gate_is_closed_over_public_operations
scope: [source]
---

# Физически адресованный нативный гейт не принимает EDT

Точный реестр зарегистрированных `NativeOperation` проходит через публичный
контекстный гейт. Физически адресованные операции принимают явный
`platform_xml` и исторический `unknown`, но отклоняют EDT и неоднозначную
классификацию; четыре логически адресованных обработчика точно перечислены как
не имеющие физического пути на этой границе.
