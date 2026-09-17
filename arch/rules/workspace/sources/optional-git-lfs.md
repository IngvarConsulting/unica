---
id: INV.SOURCE.PORTABLE-LFS-ADVISORY
check:
  - crates/unica-coder/src/infrastructure/project_health/resources.rs::project_health_repository_policy_lfs_is_advisory_for_exact_large_binary
  - crates/unica-coder/src/domain/project_health.rs::lfs_advice_is_informational_and_does_not_close_readiness
---

# Git LFS для крупных бинарных ресурсов остаётся рекомендацией

Большой бинарный ресурс без Git LFS вызывает информационную рекомендацию.
Само отсутствие LFS не делает проверку неуспешной и не закрывает
`ready` или `repositoryReady`.
