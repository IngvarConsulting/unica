---
id: DEC.2026-08-25.MODULE-PROJECTION-CORE-SLICE
status: active
governs: product
realized:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::every_approved_module_role_projects_without_a_parallel_role_registry
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::valid_missing_physical_file_keeps_possible_events_but_no_source_projection
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::common_module_requires_all_normalized_flags
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::multiline_signature_and_real_empty_body_boundaries_come_from_the_projector
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_compilation_count_and_nested_guards_match_actual_ranges
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_projection_uses_ast_and_omits_body_text
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::explicit_body_preserves_lines_paginates_and_filters_without_method_duplication
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::nested_regions_interface_projections_and_ambiguity_are_exact
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::equal_nested_region_names_are_resolved_by_full_logical_address
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::extension_annotations_resolve_independently_from_compilation_directives
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::epf_erf_and_extension_sources_project_end_to_end
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::russian_mobile_application_client_guard_normalizes_and_evaluates
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::platform_event_state_requires_exact_kind_parameter_shape_and_effective_contexts
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_catalog_is_emitted_once_for_two_bindings_on_one_owner
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_event_state_requires_exact_method_kind_and_parameter_shape
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_binding_retains_actual_element_and_nested_column_kinds
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_owner_without_bindings_still_projects_its_closed_available_catalog
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_possible_events_are_exact_for_document_and_dynamic_list_sources_without_bsl
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_event_implementation_requires_call_type_valid_for_the_form_definition
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_binding_owners_and_all_four_event_states_are_projected
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::declarative_service_projection_must_use_real_fixture_syntax
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::declarative_service_handlers_remain_on_exact_owners_without_synthetic_events
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::platform_8_3_27_module_event_catalog_is_role_specific
supersedes: []
superseded-by: null
establishes: [CTR.SOURCE.MODULE-PROJECTION-SHAPE, INV.PLATFORM.MODULE-EVENT-CATALOG]
design: docs/design/2026-08-23-v0-13-module-contract-design.md
---

# Скрытое ядро v0.13 проецирует модули и обработчики

**Решение.** Внутренний профиль v0.13 представляет допустимый модуль сводкой
и шестью отдельными проекциями `Method`, `Region`, `Interface`, `Event`,
`Compilation`, `Body`. Сводка не несёт исходник или список методов, её
счётчики совпадают с фактическими проекциями, а отсутствие физического файла
не отменяет возможные события профиля.

BSL-факты строятся из общего AST vendored parser: сигнатура, документация,
директива, накопленный препроцессор, эффективные контексты, область и
аннотация расширения не выводятся регулярными выражениями. XML `callType`
остаётся фактом привязки. Возможные события и применимость владельцу задаёт
один проверенный каталог для одобренного логического профиля 8.3.27. Применимость
событий формы использует тот же типизированный контекст, что и проверка XML:
regular/extension, вид и имя главного реквизита, родительский metadata owner и
привязка таблицы к dynamic-list source. `implemented` требует одновременно
точной BSL-декларации и допустимого для этого контекста XML `callType`.

Замороженная производная содержит фактически выведенную версию установки,
digest русского и английского Syntax Assistant 8.3.27.2074 и page ID. Полный
corpus из 693 markup-страниц (`689` event leaves и `4` structural index pages)
разбит без остатка и пересечений на события одобренного профиля и именованные
закрытые классы исключений; неизвестный owner классом исключения не становится.
Обработчики HTTP, SOAP и IntegrationService остаются на точных декларативных
владельцах и не дублируются синтетическими событиями.

Профиль явно исключает structural/generic templates без platform event ID,
ordinary-form и неподдержанные managed-form owner kinds, non-module owners и
`ExternalDataSource`: последний отсутствует в адресном профиле Task 12 и
требует отдельного архитектурного расширения, а не неявного нового owner kind.

Широкое `DEC.2026-08-23.MODULE-CONTRACT` остаётся `planned`: этот срез не
публикует `unica.view`, `unica.find`, `unica.apply` или мутацию
`event.implement`. Production-профиль v0.12 и его сериализация не меняются до
единого переключения Task 22.

**Почему.** Последующим читателям и писателю нужен один типизированный модульный
контракт до публичного cutover, иначе они независимо восстановят роли,
контексты и события.

**Цена.** До Task 22 ядро остаётся crate-private и проверяется через прямые
contract tests, а не через публичную MCP-поверхность.
