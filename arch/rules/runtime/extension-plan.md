---
id: INV.RUNTIME.EXTENSION-OPERATIONS
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::all_extension_operations_preview_then_apply_with_a_provider_receipt
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::changed_args_config_version_or_provider_never_pass_the_extension_fence
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::extension_revision_cannot_be_replayed_in_another_workspace
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::extension_arguments_are_closed_and_apply_requires_the_preview_revision
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::delete_preview_explicitly_names_the_extension_data_loss
---

# Операция над установленным расширением требует своего preview

`extensions.list`, `extensions.set` и `push` с `delete` проходят preview
и применение с `ifRev`. Это относится и к списку: получение данных базы
запускает платформенный сеанс. Preview вызывает только dry-run раннера.

Ревизия связывает рабочее пространство, аргументы, оба файла проекта,
версию раннера и квитанцию выбранного исполнителя. Изменение этих входов
останавливает применение. План удаления явно сообщает
`deletesExtensionData: true`.
