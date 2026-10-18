---
id: INV.RUNTIME.EXTENSION-OPERATIONS
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::all_extension_operations_preview_then_apply_with_a_provider_receipt
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::extension_execution_uses_current_arguments_config_and_runner
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::extension_execution_uses_its_current_workspace
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::extension_arguments_are_closed_and_execution_requires_a_boolean_mode
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::delete_preview_explicitly_names_the_extension_data_loss
---

# Операция над установленным расширением явно выбирает preview или исполнение

`extensions.list`, `extensions.set` и `push` с `delete` используют
[режим previewApply](../mcp/run-operation-names.md). Preview вызывает только
dry-run раннера. Режим обязателен и для списка: получение данных базы
запускает платформенный сеанс.

Исполнение использует текущие аргументы и состояние проекта. Ответ исполнителя должен
соответствовать операции и её цели. План удаления явно сообщает
`deletesExtensionData: true`.
