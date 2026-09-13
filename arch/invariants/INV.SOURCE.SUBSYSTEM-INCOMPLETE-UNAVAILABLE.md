---
id: INV.SOURCE.SUBSYSTEM-INCOMPLETE-UNAVAILABLE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_matches_subsystem_memberships_by_address_or_root_descriptor_uuid
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_omits_memberships_when_registered_content_reference_is_malformed
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_does_not_treat_an_unregistered_subsystem_file_as_membership
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_reports_unavailable_when_a_registered_subsystem_descriptor_is_missing
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_reports_unavailable_for_missing_or_noncanonical_subsystem_boolean
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::subsystem_evidence_cancellation_after_registered_topology_is_unavailable
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::subsystem_evidence_cancellation_after_empty_topology_is_unavailable
scope: [source]
---

# Неполное свидетельство подсистемы не становится пустой проекцией

Недопустимый зарегистрированный элемент, ошибка, отмена или неполное чтение не
публикуются как пустая доказанная проекция членства; незарегистрированный файл
также не создаёт членство.
