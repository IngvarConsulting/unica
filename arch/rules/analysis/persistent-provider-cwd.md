---
id: INV.APP.PERSISTENT-PROVIDER-CWD
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::bsl_analyzer_child_does_not_run_from_inside_the_workspace
  - crates/unica-coder/tests/platform/persistent_provider_cwd.rs::native_rlm_cwd_is_external_while_same_warm_process_survives_workspace_rename
  - crates/unica-coder/tests/platform/persistent_provider_cwd.rs::native_bsl_cwd_is_external_while_same_warm_process_survives_workspace_rename
  - crates/unica-coder/tests/platform/persistent_provider_cwd.rs::native_helper_does_not_inherit_project_cwd_from_isolated_caller
  - crates/unica-coder/tests/platform/persistent_provider_cwd.rs::windows_live_cwd_inside_outside_pair_isolates_workspace_rename
  - crates/unica-coder/src/infrastructure/workspace_services/persistent_provider_cwd_selection_tests.rs::working_directory_selection_uses_physical_candidate_and_root_identity
gap: https://github.com/IngvarConsulting/unica/issues/961
---

# Постоянный поставщик запускается вне рабочего дерева

Процесс поставщика, который остаётся жить после вызова, не получает рабочий
каталог внутри физического дерева проекта или выбранного набора исходников.
Набор исходников передаётся явно; запуск постоянного процесса не должен
удерживать дерево только из-за его `cwd`.
