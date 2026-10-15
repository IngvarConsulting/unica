---
id: INV.SURFACE.CHECK-INFERS-VALIDATORS
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_subsystem_check_validates_the_child_and_rejects_wrong_identity
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_subsystem_check_refuses_linked_dependencies
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_subsystem_check_refuses_replaced_root
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_format_owner_read_stops_at_typed_midstream_checkpoint
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_validation_keeps_captured_format_evidence_after_replacement
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_validation_does_not_follow_links_after_input_capture
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_validation_refuses_linked_format_owners_before_capture
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_dcs_check_accepts_external_report_processor_and_configuration
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_dcs_check_external_semantics_and_owner_isolation
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_dcs_check_refuses_unregistered_or_mismatched_external_template
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_dcs_check_refuses_linked_external_payload
  - crates/unica-coder/src/application/v13/check.rs::every_node_kind_owns_its_validators_without_a_caller_choice
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Вид узла определяет его валидаторы

`unica.check` принимает логический адрес `at` и сам выбирает все валидаторы
по виду и свойствам прочитанного узла. Аргумента для выбора валидатора нет.
Например, вид макета определяет проверку DCS или MXL, а модуль и тело
модуля проверяются анализатором BSL.

Ответ объединяет результаты выбранных проверок: `status`, список
`validators` и диагностики с указанием валидатора. Узел, которому валидатор
не назначен, отвечает читаемостью (`status: "readable"`). Заимствованный
узел без своего файла содержимого (`content: "inherited"`) не получает
валидатор этого файла.

Для зарегистрированной подсистемы, включая вложенную в пределах адресной
модели, проверяется выбранный дескриптор и его прямые зависимости.
Проверка родителя не заменяет проверку дочерней цели. Дескрипторы цепочки,
файлы дочерних подсистем и `CommandInterface.xml` доступны только через
удержанный корень без следования символическим ссылкам.
