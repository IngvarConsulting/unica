---
id: INV.APP.META-INFO-COVERAGE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/meta/info_projection_tests.rs::manifest_and_profile_cover_every_platform_gated_metadata_kind
scope: [app]
---

# Профиль чтения покрывает каждый вид метаданных

Манифест корпуса и активный профиль чтения совпадают с полным набором
`MetadataKind::ALL` без пропущенного или лишнего вида.
