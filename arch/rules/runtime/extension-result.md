---
id: INV.RUNTIME.EXTENSION-RESULT
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::extension_contract_rejects_wrong_subject_action_and_false_execution_claims
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::all_extension_operations_preview_then_apply_with_a_provider_receipt
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::installed_prefix_keeps_known_value_empty_value_and_unknown_distinct
---

# Ответ расширений подтверждает запрошенное действие

Ответ раннера с другим предметом или действием отклоняется. Preview
не может сообщать об уже запущенном провайдере; применение должно подтвердить
его запуск. В списке проверяются типы полей и уникальность имён.

Состояние базы приписывается провайдеру, а не независимой проверке Unica.
Префикс установленного расширения берётся из ответа провайдера о применённом
состоянии базы. Известный пустой префикс передаётся как `""`, неизвестный —
как `null`; отсутствие поля или неверный тип отклоняются. Исходники расширения
не подменяют сведения об установленном состоянии.
Сырые команды и проза планов не входят в публичный результат.
