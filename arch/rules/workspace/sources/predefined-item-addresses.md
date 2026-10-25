---
id: INV.SOURCE.PREDEFINED-ITEMS-ANSWER-BY-ADDRESS
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::predefined_items_answer_by_address_with_the_count_from_the_reader
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::predefined_items_beyond_first_page_answer_by_address
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::predefined_items_pages_preserve_complete_document_order
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::predefined_items_empty_inapplicable_and_unreadable_are_distinct
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::predefined_items_cursor_replays_after_source_removal
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::predefined_items_refuse_the_snapshot_budget_and_read_a_large_file_whole
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::predefined_items_checkpoint_preserves_cancellation_and_deadline
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::predefined_items_other_owner_fields_survive_addressed_reads
---

# Предопределённый элемент доступен по адресу независимо от страницы

Владелец предопределённых элементов сообщает их полное количество,
а не длину первой страницы. Элемент читается по имени через `PredefinedItem`
и сохраняет собственные факты: идентификатор, родителя, код, описание,
признак группы и применимые свойства вида владельца.

Список выдаётся страницами без потери хвоста. Вложенные элементы идут
в плоском документном порядке со ссылкой на родителя. Вид, у которого
предопределённых элементов быть не может, не получает эту коллекцию.
