---
id: INV.SOURCE.EXTERNAL-READ-PROFILE
status: active
governs: product
decision: DEC.2026-08-24.EXTERNAL-SOURCE-READ-PROFILE
check: crates/unica-coder/src/application/external_reader_surface_tests.rs::external_artifact_readers_share_one_logical_owner_profile
scope: [source, app, wire]
---

# Внешний source-set имеет общий логический профиль чтения

Корректный Designer EPF/ERF предоставляет одному логическому владельцу
навигацию, метаданные, форму, макет и BSL-диагностику; физический и логический
селекторы предметного читателя сходятся к тому же ресурсу, а повреждённое
дочернее доказательство не выдаётся за успешное чтение.
