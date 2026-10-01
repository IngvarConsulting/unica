---
id: INV.APP.SOURCE-SELECTION-LIMITS
check:
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_rejects_aggregate_exact_byte_budget
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::production_exact_work_can_cross_the_former_byte_ceiling
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::unlimited_exact_work_still_rejects_counter_overflow
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::repeated_exact_observations_do_not_exhaust_a_pass_work_ceiling
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_bounds_unique_retained_directories_without_ulimit
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_bounds_global_membership_across_external_source_sets
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_counts_repeated_membership_enumeration_globally
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_rejects_total_evidence_record_budget
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_rejects_route_and_name_byte_budget
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_checks_membership_budget_before_enumeration
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_checks_remaining_record_capacity_before_enumeration
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_checks_remaining_name_capacity_before_enumeration
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_rejects_before_unseen_member_child_open
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::membership_overflow_probe_never_retains_more_names_than_charged
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::membership_zero_work_rejects_before_enumeration
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::membership_child_record_cost_is_preflighted_before_open
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::membership_child_route_cost_is_preflighted_before_open
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_exact_read_never_appends_a_growth_chunk_past_the_limit
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_checks_record_budget_before_regular_open
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_deduplicates_repeated_observations
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_comparison_honors_cancellation
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_comparison_honors_deadline
---

# Проверка карты исходников имеет общий бюджет на весь проход

Один проход сохраняет не более 32 МиБ точных байтов, 65 536 записей
о наблюдаемых объектах, 128 открытых каталогов и 8 МиБ путей и имён.
Одинаковые наблюдения хранятся один раз.

В том же проходе перечисляется не более 16 384 элементов каталогов, включая
те, которые не оказались XML-файлами нужного вида. Повторное перечисление
расходует бюджет снова; новый источник не получает отдельный бюджет.
Суммарная длина файлов, намеченных для точной проверки, учитывается с
проверкой переполнения, но не служит причиной отказа по фиксированному
потолку байтов. Повторное чтение проверяет срок и отмену между блоками.

Место проверяется до чтения, перечисления, открытия следующего объекта
и увеличения буфера. Превышение даёт отказ. Сравнение собранных сведений
также останавливается при отмене или истечении срока операции.
