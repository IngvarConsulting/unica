---
id: INV.SOURCE.NO-FORMAT-MIGRATION
status: active
governs: product
decision: DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE
check: crates/unica-coder/src/application/tool_contracts.rs::native_mutation_surface_has_no_format_migration_operation_or_selector
scope: [source]
---

# Нативная поверхность не мигрирует формат

Закрытый список публичных нативных и типизированных XML-мутаторов не содержит
операции миграции или параметра целевой версии, формата либо платформы.
