---
id: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_run_dictionary.rs::development_cycle_admits_the_explicit_compatibility_subset
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

Адаптер 0.11.1 исполняет все 13 операций. Шесть операций разработки
(`push`, `pull`, `upload`, `apply`, `reset`, `infobase.create`) имеют статус
`limited` с точной схемой аргументов и описанием отсутствующих гарантий.
Их границы описывает [совместимый цикл разработки](compatible-development-cycle.md).
Старые имена не становятся алиасами.
Верхнеуровневый `infobase` поддерживает только `origin`; другая цель
не заменяется им молча. `ifRev` не подтверждает неизменность поколения базы.
