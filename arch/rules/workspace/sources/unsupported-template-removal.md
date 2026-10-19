---
id: INV.SOURCE.WHOLE-TEMPLATE-REMOVAL
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_whole_graphical_template_removal_preserves_the_exact_neighbor_and_replays
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_whole_graphical_template_removal_refuses_missing_wrapper_at_owner
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_whole_graphical_template_removal_refuses_old_body_version_at_owner
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_template_removal_active_document_absent_payload_root_at_owner
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_whole_graphical_template_removal_refuses_changed_payload_membership_and_owner
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_borrowed_template_without_type_removes_only_local_registration_and_descriptor
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_borrowed_template_foreign_owner_parent_at_owner
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_borrowed_template_saved_absence_refuses_new_empty_payload_root
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_template_removal_owner_compact_shadow_comment_at_owner
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_dcs_template_removal_clears_only_the_real_reference_and_replays
gap: https://github.com/IngvarConsulting/unica/issues/991
---

# Макет можно удалить целиком без поддержки его содержимого

Неподдерживаемое содержимое исправного макета не запрещает удалить макет
целиком. Перед применением Unica подтверждает принадлежность макета
владельцу и показывает в preview весь состав удаления и изменения владельца.
Удаляются только подтверждённые файлы выбранного макета.

Это разрешение не распространяется на редактирование неизвестного
содержимого или на попытку починить повреждённый объект.
