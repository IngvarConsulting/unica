---
id: INV.SOURCE.SUBSYSTEM-TOPOLOGY
status: active
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check:
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::pointing_at_the_subsystems_folder_answers_only_with_tree
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::concrete_subsystem_contains_its_root_chain_and_complete_descendant_tree
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::unregistered_alias_keeps_local_data_without_borrowing_a_registered_tree
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::root_subsystems_symlink_is_not_followed_for_a_tree_answer
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::nested_subsystems_symlink_is_not_followed_for_a_tree_answer
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::subsystem_info_answers_content_and_command_interface_at_once
  - crates/unica-coder/src/infrastructure/native_operations/subsystem.rs::a_missing_command_interface_is_null_not_an_empty_interface
scope: [source]
---

# Проекции подсистем выводятся из регистрации

Проекция подсистемы не имеет поля `Mode`, а её типизированный обработчик
строит дерево только из зарегистрированной топологии: выбранная подсистема
содержит цепочку предков и зарегистрированное поддерево, сохраняет `Content` и
командный интерфейс, не следует по ссылкам и не заимствует незарегистрированную
раскладку. Ошибка или срок не публикуют данные.
