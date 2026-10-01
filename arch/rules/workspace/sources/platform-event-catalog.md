---
id: INV.PLATFORM.MODULE-EVENT-CATALOG
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
---

# Доступные события соответствуют владельцу и профилю платформы

Unica выбирает возможные события из проверенного каталога платформы `8.3.27`.
Выбор зависит от роли модуля и его владельца; для формы — также от её вида,
главного реквизита и привязки таблицы к динамическому списку. События формы,
элемента, таблицы, колонки и команды сохраняют своего владельца.

У события есть уникальное имя внутри каталога, русское и английское имя
обработчика, точная сигнатура, вид метода, контексты исполнения и ссылка
на исходную страницу справки. Например, клиентское событие открытия формы
не получает серверный контекст события создания формы на сервере.

HTTP, SOAP и IntegrationService не получают вымышленных событий поверх
своих объявленных обработчиков. Обычные формы, неподдержанные владельцы,
общие шаблоны без события платформы и `ExternalDataSource` не расширяют
каталог неявно. Состав и происхождение каталога закреплены данными справки
`8.3.27.2074`; проверки сверяют эти данные и применимость событий.
