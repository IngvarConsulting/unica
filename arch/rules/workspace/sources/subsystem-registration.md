---
id: INV.SOURCE.SUBSYSTEM-REGISTRATION
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
---

# Структура подсистем строится по регистрации в XML

Построитель структуры подсистем читает `Configuration.xml` и только
дескрипторы, перечисленные в `Configuration/ChildObjects` и далее в
`Subsystem/ChildObjects`. Чтение удерживает один корень исходников и
не проходит через символические ссылки к зарегистрированным дескрипторам.
Незарегистрированные файлы и каталоги не создают узлов, не расходуют бюджет
чтения и не входят в зависимости формата.

Узлы сохраняют порядок регистрации. Ссылка `Content` должна быть адресом
метаданных или UUID; произвольная строка отклоняется. Подсистема относится
к интерфейсной роли, только если она и все её предки включены в командный
интерфейс; иначе она относится к функциональной роли.

Результат считается полным только после чтения и проверки всей
зарегистрированной структуры. Ошибка дескриптора, превышение ограничений
объёма или глубины, а также отмена не превращаются в доказанное пустое дерево.
Пустая регистрация, напротив, достаточна для пустого результата.
