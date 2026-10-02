---
id: INV.APP.META-INFO-COVERAGE
check:
  - crates/unica-coder/src/infrastructure/native_operations/meta/info_projection_tests.rs::manifest_and_profile_cover_every_platform_gated_metadata_kind
---

# Профиль чтения охватывает все объявленные виды метаданных

Профиль чтения XML платформы 8.3.27 поддерживает каждый вид из
`MetadataKind::ALL`: разбирает его свойства и дочерние элементы и строит
типизированное представление без ошибок профиля. Корпус XML содержит
те же виды без пропусков и лишних записей.

Проверка читает штатные XML каждого вида и краевые примеры из корпуса.
Она не подтверждает поддержку форматов других версий платформы.
