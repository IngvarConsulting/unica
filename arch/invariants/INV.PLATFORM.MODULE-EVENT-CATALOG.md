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
команде; применимость зависит от единого типизированного контекста Form.xml и
metadata owner, а допустимость `callType` входит в состояние binding. Для
каждого event leaf каталог фиксирует русское и английское имя обработчика,
точную декларацию, method kind, effective contexts и vendor page ID.

Именованный contract test фиксирует все 693 page ID, оба digest,
402 уникальные выбранные страницы, ровно два повторных источника проекций и
закрытые классы исключений с точными количествами. Environment-gated
`event_catalog_oracle::checked_platform_event_catalog_matches_complete_8_3_27_vendor_oracle`
дополнительно выводит версию из installation root через штатный discovery,
перечисляет полный corpus и сравнивает frozen fixture с установленным Syntax
Assistant 8.3.27.2074. Generic templates и недоступный текущему адресному
профилю `ExternalDataSource` не превращаются в события.
