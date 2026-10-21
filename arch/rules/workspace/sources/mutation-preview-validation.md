---
id: INV.SOURCE.MUTATION-PREVIEW-VALIDATION
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_creating_objects_without_the_required_final_composition
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_removing_the_last_required_register_field
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_clearing_a_web_service_namespace
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_create_then_complete_registers_previews_and_publishes_minimal_composition
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_create_then_set_namespace_previews_and_publishes_a_web_service
gap: https://github.com/IngvarConsulting/unica/issues/1937
---

# Предпросмотр проверяет будущий результат изменения

Предпросмотр правки метаданных или формы строит будущий результат
и выполняет его предметную проверку.
При неизменных входных данных ошибка результата не может обнаруживаться
только после разрешения записи. Предпросмотр не записывает файлы.

Для метаданных проверен канонический `apply`: предпросмотр недопустимого
итогового объекта отказывает по предметной причине и не пишет исходники,
а сохранённый предпросмотр допустимого объекта публикуется тем же планом. Для формы
таких проверок на каноническом пути пока нет; это разрыв. Проверки не
доказывают все операции канонического `apply`.
Общие гарантии его подготовки и публикации заданы отдельными правилами.
