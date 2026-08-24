---
id: INV.PLATFORM.MODULE-EVENT-CATALOG
status: active
governs: product
decision: DEC.2026-08-25.MODULE-PROJECTION-CORE-SLICE
check: crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::platform_8_3_27_module_event_catalog_is_role_specific
scope: [platform, product, source]
---

# Возможные события принадлежат единому профилю платформы 8.3.27

Каталог различает применимость для всех семантических ролей Task 12; HTTP,
SOAP и IntegrationService не получают синтетические события поверх своих
декларативных владельцев, Bot и WebSocketClient сохраняют отдельные события.
События формы остаются на форме, элементе, таблице, вложенной колонке или
команде и переиспользуют те же матрицы, что действующая валидация формы.
