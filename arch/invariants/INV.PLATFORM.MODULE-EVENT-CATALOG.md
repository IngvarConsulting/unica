---
id: INV.PLATFORM.MODULE-EVENT-CATALOG
status: active
governs: product
decision: DEC.2026-08-25.MODULE-PROJECTION-CORE-SLICE
check:
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::module_event_applicability_covers_every_approved_role_family
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::module_catalog_covers_the_task12_direct_owner_role_matrix_exactly
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::form_event_applicability_preserves_every_logical_owner_family
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::form_applicability_variants_have_exact_closed_event_additions_and_counts
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::event_catalog_entries_have_exact_bilingual_shape_context_and_provenance
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::every_catalog_has_unique_semantic_event_ids_not_generic_storage_names
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::form_catalog_execution_contexts_distinguish_client_and_server_callbacks
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::checked_event_catalog_is_a_closed_immutable_8_3_27_set
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::checked_event_fixture_is_non_skipping_closed_partition_evidence
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
