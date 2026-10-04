---
id: INV.SOURCE.RETAINED-LOGICAL-PUBLICATION
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::logical_read_admission_does_not_scan_source_revisions
  - crates/unica-coder/src/infrastructure/daemon/server.rs::logical_read_publication_does_not_scan_source_revisions
  - crates/unica-coder/src/infrastructure/daemon/server.rs::logical_read_parent_publication_deadline_discards_staged_extension_data
  - crates/unica-coder/src/infrastructure/daemon/server.rs::logical_reads_preserve_deadline_without_source_scans_or_mutation_lane_wait
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_module_reader_never_follows_a_source_set_remap
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_typed_reader_remains_on_the_admitted_root_after_source_set_remap
---

# Чтение удерживает выбранный корень без ревизии дерева

`view`, `resolve`, `search`, `check` и `diff` не снимают и не проверяют общую
ревизию исходников ни при допуске, ни при выдаче ответа. Чтение использует
выбранный удержанный корень, не следует вложенным ссылкам и не принимает
подмену корня. Исходный срок и отмена продолжают действовать при передаче
работы между исполнителями. Выдача ответа не ждёт очередь записи актора.

Ответ чтения не обещает атомарный снимок всей конфигурации. Для применения
правки требуется отдельный dry-run с маркером конкретного плана. Готовность
поискового индекса не доказывает актуальность исходников; индекс сообщает
своё поколение и неизвестную актуальность. Страницы сохранённого результата
продолжают этот результат, а новый запрос читает доступные текущие входы.
