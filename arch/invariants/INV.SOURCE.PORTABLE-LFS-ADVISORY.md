---
id: INV.SOURCE.PORTABLE-LFS-ADVISORY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/project_health/resources.rs::portable_lfs_advice_and_readiness_contract_is_complete
scope: [source]
---

# LFS остаётся необязательной подсказкой

Большой бинарный ресурс без Git LFS получает рекомендацию, но проверка остаётся
успешной и не закрывает `repositoryReady` или `ready`.
