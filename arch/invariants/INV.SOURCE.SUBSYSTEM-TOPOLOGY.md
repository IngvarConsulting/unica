---
id: INV.SOURCE.SUBSYSTEM-TOPOLOGY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/application/mod.rs::public_subsystem_info_registration_address_and_schema_contract_is_complete
  - crates/unica-coder/src/application/mod.rs::public_subsystem_info_projects_registered_dependency_errors_as_typed_failures
  - crates/unica-coder/src/application/mod.rs::public_subsystem_info_deadline_returns_no_data
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::pointing_at_the_subsystems_folder_answers_only_with_tree
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::concrete_subsystem_contains_its_root_chain_and_complete_descendant_tree
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::unregistered_alias_keeps_local_data_without_borrowing_a_registered_tree
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::root_subsystems_symlink_is_not_followed_for_a_tree_answer
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::nested_subsystems_symlink_is_not_followed_for_a_tree_answer
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::subsystem_info_answers_content_and_command_interface_at_once
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::a_missing_command_interface_is_null_not_an_empty_interface
scope: [source]
---

# Публичные проекции подсистем выводятся из регистрации

Публичный `subsystem.info` не имеет поля `Mode`, а его типизированный обработчик
строит дерево только из зарегистрированной топологии: выбранная подсистема
содержит цепочку предков и зарегистрированное поддерево, сохраняет `Content` и
командный интерфейс, не следует по ссылкам и не заимствует незарегистрированную
раскладку. Ошибка или срок не публикуют данные.
