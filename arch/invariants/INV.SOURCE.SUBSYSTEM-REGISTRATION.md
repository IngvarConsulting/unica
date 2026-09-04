---
id: INV.SOURCE.SUBSYSTEM-REGISTRATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::registration_order_drives_roles_and_interface_membership
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::registered_dependency_paths_follow_registration_order_exactly
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::content_references_are_typed_and_match_both_descriptor_identities
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::arbitrary_nonempty_content_reference_rejects_the_topology
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::unregistered_files_do_not_define_or_break_the_topology
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::unregistered_oversized_xml_does_not_spend_the_topology_byte_budget
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::unregistered_file_symlink_does_not_affect_the_topology
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::unregistered_directory_symlink_branch_does_not_affect_the_topology
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::registered_oversized_descriptor_fails_closed
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::missing_malformed_and_duplicate_registered_nodes_are_rejected
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::empty_registration_proves_an_empty_topology_without_a_subsystems_directory
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::ninth_registered_level_exceeds_the_shared_address_budget
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::complete_result_requires_a_checkpoint_after_secure_capture_and_parsing
  - crates/unica-coder/src/infrastructure/subsystem_topology.rs::registered_descriptor_symlink_is_not_followed
scope: [source]
---

# Топология подсистем выводится только из регистрации

Единый построитель под одним удерживаемым корнем, открытым без перехода по
символическим ссылкам, читает `Configuration.xml` и только транзитивно
зарегистрированные дескрипторы из `Configuration/ChildObjects` и
`Subsystem/ChildObjects`. Только они расходуют бюджеты и образуют зависимости
формата, незарегистрированная раскладка не влияет на доказательство, каждый
элемент `Content` имеет тип `MetadataAddress | UUID`, а каждый доказанный узел
принадлежит ровно одной эффективной роли.
