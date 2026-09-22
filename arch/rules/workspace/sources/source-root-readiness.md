---
id: INV.SOURCE.ROOT-SEPARATION
check:
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::root_identity_equal_to_workspace_is_one_primary_fact
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::linked_alias_to_workspace_reports_the_primary_identity_cause_with_evidence
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::outside_source_root_is_a_typed_unsafe_path
  - crates/unica-coder/tests/platform/project_health.rs::project_health_workspace_root_rejection_suppresses_source_derived_git_facts
---

# Проверка готовности требует отдельный корень исходников внутри проекта

При проверке готовности корень набора исходников должен быть вложенным
каталогом проекта. Если он совпадает с корнем проекта, Unica сообщает
`source_set.root_is_workspace` и `ready: false`. Записи `.`, `./`, `src/..`
и ссылка на корень проекта не обходят эту проверку.

Совпадение даёт одну первичную причину отказа; для ссылки в ответе указан
её путь. Проверки служебных файлов и Git, зависящие от корректности этого
корня, не запускаются и не создают повторные ошибки. Проверка не меняет
рабочее дерево.
