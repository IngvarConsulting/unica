---
id: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_infobase_create_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_cf_import_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_source_export_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_source_import_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_run_dictionary.rs::runner_one_refuses_unsupported_semantics_and_old_names_before_admission
  - crates/unica-coder/src/infrastructure/daemon/v13_run_dictionary.rs::runner_one_never_redirects_an_unsupported_infobase_to_origin
  - crates/unica-coder/src/infrastructure/daemon/v13_run_dictionary.rs::limited_delete_and_known_operations_reach_their_typed_parsers
---

# Известная операция не обещает поддержку всех её режимов

`support.state` различает `supported`, `limited` и `unavailable`.
Ограниченная операция сообщает адаптер, причину и `supportedArgs`.
Неподдерживаемый запрос получает отказ до допуска исходников и запуска
платформы, а не успешный preview.

Адаптер 0.11 разрешает `push` только для удаления установленного расширения.
Обычная отправка, `pull`, `upload`, `apply`, `reset` и `infobase.create`
недоступны. Старые обработчики не доказывают их целевую семантику.
Верхнеуровневый `infobase` поддерживает только `origin`; другая цель
не заменяется им молча. `ifRev` не подтверждает неизменность поколения базы.
