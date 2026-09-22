---
id: INV.SOURCE.RETAINED-LOGICAL-PUBLICATION
check:
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_snapshot_reuses_a_clean_fence_and_reconciles_once_after_change
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_root_replacement_during_retained_scan
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rechecks_replaced_nested_directory
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rechecks_replaced_file
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_membership_added_after_enumeration
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_in_place_change_after_hash
  - crates/unica-coder/src/infrastructure/source_revision.rs::unsupported_fence_stable_operation_lease_scans_at_admission_and_confirmation
  - crates/unica-coder/src/infrastructure/source_revision.rs::unsupported_fence_reconcile_is_bounded_to_six_passes_when_corpus_never_stabilizes
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_scan_limits_entries_files_and_aggregate_bytes
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_rejects_revision_change_during_post_fence_owner_proof
gap: https://github.com/IngvarConsulting/unica/issues/971
---

# Результат чтения выдаётся после повторной проверки исходников

Перед выдачей результата чтения Unica повторно подтверждает состояние
исходников. Подмена корня, изменение состава каталога или прочитанных байтов
во время этой проверки отклоняет результат. Это включает замену вложенного
каталога или файла экземпляром с другим содержимым.
Если заменён вложенный каталог или файл, но логические пути, состав дерева
и учитываемые байты остались прежними, результат разрешён после повторного
подтверждения новых экземпляров. Срок и число попыток не увеличиваются.
Это относится только к чтению: проверки перед записью не ослабляются,
а совпадение байтов не разрешает подмену корня, ссылки или выход за границы.
Проверки подмены открытого каталога выполняются на ОС, которые её допускают.

Если платформенное наблюдение подтверждает отсутствие изменений,
сервис ревизий использует сохранённую ревизию без повторного полного обхода.
После стабилизировавшегося изменения содержимого достаточно одного
повторного обхода; он возвращает новую ревизию.

Если платформа не предоставляет надёжного подтверждения изменений,
начальный допуск и конечное подтверждение каждого выбранного набора
требуют по два совпавших обхода.
Сравниваются содержимое и физическая идентичность файлов и каталогов.
Стабильная операция требует четырёх обходов каждого выбранного набора.
На каждой из двух границ — допуск и подтверждение — допускается не более
трёх попыток по два обхода.
Каждый обход ограничивает число элементов, размер одного файла и общий
объём читаемых байтов.

Это проверка наблюдаемого состояния: она не гарантирует обнаружение
изменения, которое произошло и было отменено между наблюдениями.

Подготовка `apply`, включая допуск и предпросмотр, не ослабляет эти проверки
и не отключает повторное использование подтверждённой ревизии. Проверка
последующего чтения на том же акторе пока не закрыта; сценарии указаны в `gap`.
