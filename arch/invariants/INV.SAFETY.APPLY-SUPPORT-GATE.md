---
id: INV.SAFETY.APPLY-SUPPORT-GATE
status: active
governs: product
decision: DEC.2026-09-03.APPLY-SUPPORT-GATE
check: crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::locked_vendor_objects_refuse_every_family_before_staging
scope: [app, source]
---

# Канонический `apply` отказывает запертому объекту поставщика до первого байта

Планировщик `unica.apply` проверяет правило поддержки владельца адресованного
узла до того, как любое семейство операций положит байт в staged state:
объект поставщика с правилом «не редактируется» и конфигурация с запрещёнными
изменениями отвечают `invalid_state`. Закрытое исключение — операции самой
поддержки (`supportCapability.set`, `supportRule.set`). Превью и публикация
отказывают одинаково; корпус приёмки замораживает ответ на объекте поставщика
фикстуры.
