---
id: INV.SOURCE.RETAINED-LOGICAL-PUBLICATION
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::logical_read_of_one_object_opens_only_its_own_files
  - crates/unica-coder/src/infrastructure/daemon/server.rs::borrowed_logical_read_admits_and_publishes_without_unrelated_objects
  - crates/unica-coder/src/infrastructure/daemon/server.rs::logical_reads_run_without_deadline_tree_scans_or_mutation_lane_wait
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_module_reader_never_follows_a_source_set_remap
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_typed_reader_remains_on_the_admitted_root_after_source_set_remap
---

# Чтение удерживает выбранный корень без ревизии дерева

`view`, `resolve`, `search`, `check` и `diff` не снимают и не проверяют общую
ревизию исходников ни при допуске, ни при выдаче ответа. Чтение использует
выбранный удержанный корень, не следует вложенным ссылкам и не принимает
подмену корня. Срока операции у чтения нет; отмена продолжает действовать
при передаче работы между исполнителями. Выдача ответа не ждёт очередь
записи актора.

Ответ чтения не обещает атомарный снимок всей конфигурации. Для применения
правки требуются
[предпросмотр и исполнение сохранённого плана](../apply-revision-fence.md).
Готовность поискового индекса не доказывает актуальность исходников; индекс сообщает
своё поколение и неизвестную актуальность. Страницы сохранённого результата
продолжают этот результат, а новый запрос читает доступные текущие входы.
