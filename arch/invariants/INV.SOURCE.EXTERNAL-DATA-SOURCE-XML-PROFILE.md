---
id: INV.SOURCE.EXTERNAL-DATA-SOURCE-XML-PROFILE
status: active
governs: product
decision: DEC.2026-08-22.EXTERNAL-DATA-SOURCE-METADATA
check: crates/unica-coder/src/infrastructure/native_operations/meta/template_catalog_tests.rs::typed_minimal_external_data_source_matches_platform_8_3_27_shape
scope: [platform, source]
---

# Минимальный внешний источник следует профилю 8.3.27

Эмиттер создаёт `ExternalDataSource` версии 2.20 с базовыми свойствами,
`DataLockControlMode`, `ChildObjects` и категориями `Manager`, `TablesManager`,
`CubesManager` в доказанном платформенном порядке.
