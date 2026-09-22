---
id: INV.RUNTIME.EXTENSION-RESULT
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::extension_contract_rejects_wrong_subject_action_and_false_execution_claims
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::all_extension_operations_preview_then_apply_with_a_provider_receipt
---

# Ответ расширений подтверждает запрошенное действие

Ответ раннера с другим предметом или действием отклоняется. Preview
не может сообщать об уже запущенном провайдере; применение должно подтвердить
его запуск. В списке проверяются типы полей и уникальность имён.

Состояние базы приписывается провайдеру, а не независимой проверке Unica.
Префикс расширения берётся из исходников и не выдумывается для inventory.
Сырые команды и проза планов не входят в публичный результат.
