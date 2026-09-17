---
id: INV.SOURCE.PORTABLE-GIT
check:
  - crates/unica-coder/tests/platform/project_health.rs::portable_git_readiness_contract_is_a_closed_positive_and_negative_matrix
  - crates/unica-coder/src/infrastructure/project_health/git.rs::project_health_git_untracked_gitignore_is_local_only
  - crates/unica-coder/src/infrastructure/project_health/git.rs::project_health_git_ignore_uses_staged_empty_file_not_valid_worktree_file
  - crates/unica-coder/src/domain/project_health.rs::project_health_serializes_independent_source_and_repository_readiness
  - crates/unica-coder/src/domain/project_health.rs::incomplete_repository_fact_serializes_its_check_as_not_run
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_separates_source_and_repository_readiness
---

# Готовность репозитория подтверждается переносимыми настройками Git

`repositoryReady` вычисляется отдельно от готовности исходников `ready`.
Для него нужны отслеживаемые правила исключений и атрибутов, соответствующие
виду ресурсов, допустимые окончания строк и безопасно распознанный
`ConfigDumpInfo.xml`, подготовленный к коммиту.

Корректные исходники без Git дают `ready: true`, `repositoryReady: false`
и отдельную диагностику `git.repository_absent` в корневом `unica.check {}`.

Проверяется версия правил в индексе Git. Локальные настройки и ещё
не добавленная в индекс правка не заменяют переносимые правила.
Ошибка или неполное выполнение обязательной проверки не дают
`repositoryReady: true`. [Рекомендация Git LFS](optional-git-lfs.md)
не относится к обязательным проверкам готовности.
