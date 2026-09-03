---
id: INV.SOURCE.CODE-PATCH-EOL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_observed_eol_policy_is_closed
scope: [product, source]
---

# Code patch сохраняет наблюдаемую локальную форму EOL

`unica.code.patch` сохраняет нетронутые байты смешанного файла и использует EOL
целевого метода, обслуживает источник без EOL явным LF, сохраняет отсутствие
завершающего перевода и отказывает на одиночном CR вместо глобальной
нормализации.
