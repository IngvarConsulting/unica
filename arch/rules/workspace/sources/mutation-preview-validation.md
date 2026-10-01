---
id: INV.SOURCE.MUTATION-PREVIEW-VALIDATION
check:
  - crates/unica-coder/src/infrastructure/native_operations/cf.rs::cf_init_preview_shares_the_apply_data_shape_and_writes_nothing
  - crates/unica-coder/src/infrastructure/native_operations/cf.rs::cf_init_preview_rejects_what_apply_rejects_and_writes_nothing
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::typed_edit_preview_bytes_equal_the_applied_post_image
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::form_edit_preview_apply_and_no_op_validate_the_projected_form
gap: https://github.com/IngvarConsulting/unica/issues/959
---

# Предпросмотр проверяет будущий результат изменения

Предпросмотр создания конфигурации и правки метаданных или формы
строит будущий результат и выполняет его предметную проверку.
При неизменных входных данных ошибка результата не может обнаруживаться
только после разрешения записи. Предпросмотр не записывает файлы.

Отрицательные сценарии проверены для конфигурации и формы. Для метаданных
проверено совпадение подготовленных и опубликованных байтов; отказ самого
предпросмотра ещё требует проверки. Проверки относятся к внутренним
генераторам и обработчикам этих трёх семейств. Они не доказывают все операции канонического `apply`.
Общие гарантии его подготовки и публикации заданы отдельными правилами.
