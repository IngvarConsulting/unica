---
id: INV.APP.RETAINED-SOURCE-SELECTION-FINALITY
check:
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_rejects_inconsistent_regular_repeat
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_rejects_inconsistent_directory_repeat
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_rejects_inconsistent_membership_repeat
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_v8project_kind_change_after_prepare
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_v8project_absence_to_appearance_after_prepare
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_autodetected_extension_membership_change
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_unselected_declared_parent_appearance
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_unselected_non_platform_map_input_change
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_repaired_oversized_unselected_external_descriptor
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_autodetection_container_identity_replacement
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_admission_external_config_dump_info_content_change_invalidates_evidence
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_admission_external_descriptor_absence_to_appearance_invalidates_evidence
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_dry_run_rejects_late_map_change_without_receipt
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_late_change_rolls_back_source_cache_revision_and_receipt
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::published_replacement_of_a_retained_source_map_file_passes_the_final_gate
gap: https://github.com/IngvarConsulting/unica/issues/987
---

# Изменение применяется к той карте исходников, по которой его подготовили

Карта исходников принимается после двух совпавших проходов: совпасть должны
и параметры источников, и сохранённые сведения о файлах и каталогах.
Учитываются все входы карты, включая невыбранные и неподдерживаемые источники.
Сохраняются точные байты `v8project.yaml` и содержательно разобранных
`ConfigDumpInfo.xml`, отсутствие или вид ожидаемых объектов, физическая
идентичность и состав просмотренных каталогов. Слишком большой дескриптор
повторно проверяется как тот же файл, всё ещё превышающий допустимый размер.

`apply` повторно проверяет эти сведения двумя проходами перед записью,
перед результатом `dryRun` и после записи. Постороннее изменение даёт отказ;
если запись уже началась, восстанавливаются исходники, кеш и состояние ревизии.
Подмена файла самой подготовленной операцией допустима, когда его новые байты
совпадают с планом, а сама карта источников осталась прежней.

Это проверка состояния в заданных точках. Она не обнаруживает изменение,
которое успели полностью отменить между проверками.

Если маркер нужен только для проверки существования, изменение его байтов
на месте не меняет карту. Его исчезновение, смена вида или идентичности
по-прежнему значимы. Для содержательно читаемых входов сравниваются байты.

Проба без завершённого наблюдения закрывает допуск: ошибку нельзя принять
за доказанное отсутствие файла. Проверка этих двух условий на полном пути
актора остаётся в `gap`.
