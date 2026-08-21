---
id: INV.SOURCE.PORTABLE-LFS-ADVISORY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/project_health/resources.rs::project_health_repository_policy_lfs_is_advisory_for_exact_large_binary
scope: [source]
---

# LFS остаётся необязательной подсказкой

Большой бинарный ресурс без Git LFS получает рекомендацию, но проверка остаётся
успешной и не закрывает `repositoryReady` или `ready`.
