---
id: INV.RUNTIME.EXTENSION-OPERATIONS
status: active
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::changed_args_config_version_or_provider_never_pass_the_extension_fence
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::extension_revision_cannot_be_replayed_in_another_workspace
scope: [wire, product]
---

# Операции расширений применяют только просмотренный план

Рабочее пространство, аргументы, оба файла проекта, версия раннера и provider receipt связывают
применение с preview через ifRev. Несовпадение останавливает вызов до apply.

Превью `push` с `delete` явно называет удаление данных расширения
через `deletesExtensionData: true`.
