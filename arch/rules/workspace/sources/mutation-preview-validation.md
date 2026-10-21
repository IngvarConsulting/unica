---
id: INV.SOURCE.MUTATION-PREVIEW-VALIDATION
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_creating_objects_without_the_required_final_composition
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_removing_the_last_required_register_field
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_clearing_a_web_service_namespace
  - crates/unica-coder/src/infrastructure/workspace_actor_contract_tests.rs::prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write
gap: https://github.com/IngvarConsulting/unica/issues/1937
---

# Предпросмотр проверяет будущий результат изменения

Предпросмотр правки метаданных или формы строит будущий результат
и выполняет его предметную проверку.
При неизменных входных данных ошибка результата не может обнаруживаться
только после разрешения записи. Предпросмотр не записывает файлы.

Для метаданных проверен канонический `apply`: предпросмотр недопустимого
итогового объекта отказывает по предметной причине и не пишет исходники,
а пробный прогон возвращает прогнозируемый результат без записи. Для формы
таких проверок на каноническом пути пока нет; это разрыв. Проверки не
доказывают все операции канонического `apply`.
Общие гарантии его подготовки и публикации заданы отдельными правилами.
