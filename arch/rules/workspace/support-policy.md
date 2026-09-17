---
id: INV.SAFETY.SUPPORT-POLICY-DOWNGRADE
check:
  - crates/unica-coder/src/infrastructure/support_guard.rs::project_editing_policy_is_the_closed_support_guard_downgrade_source
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_preserves_workspace_ancestor_precedence_over_source_local_policy
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::support_policy_candidate_search_stops_at_exact_twentieth_candidate
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::support_policy_overlapping_chains_keep_first_occurrence_order_without_duplicates
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::support_policy_database_paths_distinguish_nested_sources_from_prefix_siblings
---

# Ослабить блокировку поддержки можно только политикой проекта

Реакцию на обнаруженный запрет редактирования ослабляет только
`editingAllowedCheck` со значением `warn` или `off` в выбранном
`.v8-project.json`. Отсутствующий или повреждённый файл, пропущенное поле
и неизвестное значение сохраняют режим `deny`.

Поиск начинается от корня проекта вверх: не более 20 каталогов, включая
начальный. Затем так же проверяется каталог исходников и его предки;
повторяющиеся пути пропускаются. Первый найденный `.v8-project.json`
завершает поиск, даже если прочитать его как файл политики нельзя.

Настройка в `databases` применяется к указанному `configSrc` и вложенным
исходникам. Например, `src-copy` не попадает под настройку для `src`.
Изменение самой поддержки объекта — отдельная операция, а не ещё один
источник этой настройки.
