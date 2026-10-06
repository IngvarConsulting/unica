---
id: INV.APP.SOURCE-SELECTION-LIMITS
check:
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::oversized_invalid_project_config_is_a_typed_yaml_refusal
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::project_config_past_eight_mib_keeps_its_exact_useful_root
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::project_config_past_eight_mib_rejects_malformed_tail_and_extra_document
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::source_layout_inspects_all_1025_declared_source_roots
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::health_source_layout_keeps_all_defaulted_rows_and_their_real_ambiguity
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::health_yaml_aliases_keep_exact_selected_values_and_real_name_ambiguity
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::health_yaml_many_selected_container_nodes_keep_the_useful_layout
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::health_yaml_aliased_root_key_keeps_the_useful_layout_after_many_selected_nodes
  - crates/unica-coder/src/infrastructure/project_sources.rs::controlled_health_keeps_format_evidence_past_four_mib_below_the_old_entry_limit
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_health_keeps_format_evidence_past_four_mib_below_the_old_entry_limit
  - crates/unica-coder/src/infrastructure/project_sources.rs::controlled_health_external_descriptor_survives_more_than_16384_siblings
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_health_external_descriptor_survives_more_than_16384_siblings_and_binds_membership
  - crates/unica-coder/src/infrastructure/project_sources.rs::injected_health_evidence_entry_allowance_refuses_without_partial_results
  - crates/unica-coder/src/infrastructure/project_sources.rs::injected_health_evidence_byte_allowance_refuses_without_partial_results
  - crates/unica-coder/src/infrastructure/project_sources.rs::controlled_health_accepts_yaml_input_past_eight_mib
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_health_accepts_yaml_input_past_eight_mib_and_binds_late_bytes
  - crates/unica-coder/src/infrastructure/project_sources.rs::controlled_health_accepts_1025_declared_source_sets
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_health_accepts_1025_declared_source_sets
  - crates/unica-coder/src/infrastructure/project_sources.rs::controlled_health_autodetects_1025_source_directories
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_health_autodetects_1025_source_directories
  - crates/unica-coder/src/infrastructure/project_sources.rs::health_yaml_selected_values_past_two_mib_keep_their_useful_source
  - crates/unica-coder/src/infrastructure/project_sources.rs::health_yaml_visitor_retains_a_selected_value_past_two_mib
  - crates/unica-coder/src/infrastructure/project_sources.rs::health_yaml_ignored_nodes_past_65536_keep_their_useful_source
  - crates/unica-coder/src/infrastructure/project_sources.rs::health_yaml_ignored_alias_bytes_past_sixteen_mib_keep_their_useful_source
  - crates/unica-coder/src/infrastructure/project_sources.rs::health_yaml_ignored_depth_past_256_keeps_its_useful_source
  - crates/unica-coder/src/infrastructure/project_sources.rs::health_yaml_ignored_alias_depth_past_256_keeps_its_useful_source
  - crates/unica-coder/src/infrastructure/project_sources.rs::health_yaml_keeps_route_specific_duplicate_and_anchor_semantics
  - crates/unica-coder/src/infrastructure/project_sources.rs::health_yaml_rejects_malformed_tail_and_extra_document_after_useful_source
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::aggregate_selection_directory_handles_cross_former_128_and_keep_finality
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::aggregate_selection_exact_bytes_cross_former_32mib_without_source_walk
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::aggregate_selection_membership_crosses_former_16384_across_sources
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::aggregate_selection_records_cross_former_65536_with_real_absences
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::aggregate_selection_route_names_cross_former_8mib_with_real_absences
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::unbounded_selection_counters_refuse_overflow_without_mutation
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::production_exact_work_can_cross_the_former_byte_ceiling
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::unlimited_exact_work_still_rejects_counter_overflow
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::repeated_exact_observations_do_not_exhaust_a_pass_work_ceiling
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_exact_read_never_appends_a_growth_chunk_past_the_limit
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_deduplicates_repeated_observations
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_comparison_honors_cancellation
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_comparison_honors_deadline
---

# Проверка карты исходников не отказывает по суммарным квотам прохода

Суммарные точные байты входов карты, записи наблюдений, перечисленные элементы,
открытые каталоги и пути с именами учитываются без фиксированного потолка.
Корректное наблюдение за прежними порогами продолжает работу. Одинаковые
наблюдения хранятся один раз; повторное чтение и перечисление сохраняют
проверки отмены и согласованности входов.

Проверка состояния проекта и допуск операции читают весь `v8project.yaml`
и все объявленные или автоматически найденные наборы исходников без квот
размера файла и количества наборов. Неизвестные поля YAML пропускаются;
их вложенность и логический размер раскрытых алиасов не вводят отдельного
отказа. Известные поля сохраняют прежние правила типов и дублей. Неразрешённые
алиасы и незавершённый документ по-прежнему дают отказ.

Свидетельства формата каждого набора сохраняются без суммарной квоты
числа записей и байтов. Поиск XML-дескриптора внешней обработки или отчёта
перечисляет все непосредственные элементы каталога, включая соседей,
которые не являются XML. При отсутствии квоты перечисление также не получает
числового потолка. Небезопасные пути не обходятся, а действительная ошибка
доступа сохраняет причину отказа. Неоднозначные имена наборов остаются
неоднозначными независимо от их количества.

Подсчёт проверяет арифметическое переполнение. Ошибки доступа, диска и
открытия дескрипторов ОС сохраняют отказ с причиной. Обнаруженное изменение
карты также даёт отказ по [правилу финальной проверки](source-selection-finality.md).
Это входы выбора источника, а не снимок всего BSL-кода проекта.

Подготовка по-прежнему сохраняет точные байты читаемых входов и метаданные,
в том числе весь список свидетельств формата, в памяти и использует два прохода.
Значения известных полей разбираются
существующей библиотекой YAML; её ограничения рекурсивного построения
значения и раскрытия алиасов остаются. Полное чтение конфигурации для runtime
также пока исправляется в
https://github.com/IngvarConsulting/unica/issues/1119. Конечный срок, переданный
существующим вызывающим кодом, пока также проверяется; отсутствие автоматического
срока проводится через весь runtime в той же задаче.
