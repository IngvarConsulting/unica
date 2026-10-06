---
id: INV.APP.SOURCE-SELECTION-LIMITS
check:
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

Подсчёт проверяет арифметическое переполнение. Ошибки доступа, диска и
открытия дескрипторов ОС сохраняют отказ с причиной. Обнаруженное изменение
карты также даёт отказ по [правилу финальной проверки](source-selection-finality.md).
Это входы выбора источника, а не снимок всего BSL-кода проекта.

Подготовка по-прежнему сохраняет точные байты читаемых входов и метаданные
в памяти и использует два прохода. Эта запись снимает пять суммарных квот;
отдельные ограничения чтения файла и разбора карты пока снимаются в
https://github.com/IngvarConsulting/unica/issues/1119. Конечный срок, переданный
существующим вызывающим кодом, пока также проверяется; отсутствие автоматического
срока проводится через весь runtime в той же задаче.
