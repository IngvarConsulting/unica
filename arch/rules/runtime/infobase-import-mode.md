---
id: INV.RUNTIME.INFOBASE-IMPORT-MODE
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::restore_arguments_require_a_stated_mode_and_a_dt_input
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::restore_preview_names_the_source_and_the_mode_without_touching_the_infobase
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::restore_apply_attributes_the_infobase_state_to_the_provider
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::restore_apply_refuses_when_the_provider_reports_another_mode
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::restore_refuses_a_preview_that_claims_it_already_restored
---

# Загрузка DT требует явного режима и называет источник сведений о базе

`infobase.restore` требует входной DT и явный режим `create` или `replace`.
Аргументы `output` и `connection` не принимаются. Preview не выполняет
загрузку и передаёт тот же вход и режим в следующий вызов; ответ раннера
с другим режимом или уже выполненной загрузкой отвергается.

После применения результат называет режим и состояние базы,
засвидетельствованное провайдером. Он отмечает изменение базы, а не создание
артефакта или перезапись DT. Preview явно сообщает, что состояние базы
до применения ему неизвестно.

Проверки исполняют адаптер с управляемыми ответами раннера. Они не проверяют
реальное состояние базы платформой 1С.
