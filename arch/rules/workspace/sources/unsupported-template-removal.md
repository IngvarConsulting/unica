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
  - crates/unica-coder/src/domain/apply.rs::template_format_is_a_separate_filter_for_dcs_and_mxl_families
  - tests/ci/test_whole_template_plan_diff.py::WholeTemplatePlanDiffWireTests.test_complete_removal_pages_define_the_same_token_publication
  - tests/ci/test_whole_template_plan_diff.py::WholeTemplatePlanDiffWireTests.test_owner_mutation_makes_diff_and_apply_stale_without_overwriting_it
  - tests/ci/test_whole_template_plan_diff.py::WholeTemplatePlanDiffWireTests.test_file_membership_mutation_makes_diff_and_apply_stale_without_overwriting_it
  - tests/ci/test_whole_template_plan_diff.py::WholeTemplatePlanDiffWireTests.test_picture_bytes_mutation_makes_diff_and_apply_stale_without_overwriting_it
---

# Макет можно удалить целиком без поддержки его содержимого

Неподдерживаемое содержимое исправного макета не запрещает удалить макет
целиком. Перед применением Unica подтверждает принадлежность макета
владельцу и показывает в preview весь состав удаления и изменения владельца.
Удаляются только подтверждённые файлы выбранного макета.

Это разрешение не распространяется на редактирование неизвестного
содержимого или на попытку починить повреждённый объект.
