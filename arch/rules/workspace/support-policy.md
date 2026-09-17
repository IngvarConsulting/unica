---
id: INV.SAFETY.SUPPORT-POLICY-DOWNGRADE
check:
  - crates/unica-coder/src/infrastructure/support_guard.rs::project_editing_policy_is_the_closed_support_guard_downgrade_source
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_preserves_workspace_ancestor_precedence_over_source_local_policy
---

# Ослабить блокировку поддержки можно только политикой проекта

Реакцию на обнаруженный запрет редактирования ослабляет только
`editingAllowedCheck` со значением `warn` или `off` в выбранном
`.v8-project.json`. Отсутствующий или повреждённый файл, пропущенное поле
и неизвестное значение сохраняют режим `deny`.

Политика у корня проекта или его предков имеет приоритет перед политикой
в каталоге исходников. Изменение самой поддержки объекта — отдельная
операция, а не ещё один источник этой настройки.
