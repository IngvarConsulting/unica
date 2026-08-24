---
id: DEC.2026-08-24.EXTERNAL-SOURCE-READ-PROFILE
status: active
governs: product
realized: crates/unica-coder/src/application/external_reader_surface_tests.rs::external_artifact_readers_share_one_logical_owner_profile
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.EXTERNAL-READ-PROFILE]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-08-24-external-source-read-profile-design.md
---

# Внешние артефакты входят в логический профиль чтения

**Решение.** Read-only resolver принимает Platform XML source-set внешних
обработок и отчётов и доказывает их объекты, формы, макеты и модули через
дескриптор-владелец. Writer-профиль остаётся ограничен Configuration и
Extension. Предметные читатели и диагностика используют read-only resolver;
`meta.info` сохраняет внешний discriminator и не требует регистрационную
`Configuration.xml`, а ошибки доказательств дочерних ресурсов остаются видимы.

**Почему.** Один корректный дамп должен быть адресуемым для validate и outline,
а требование способности к записи не должно блокировать обязательные
read-only readers.

**Цена.** Внешняя раскладка имеет виртуальный корень, поэтому точное чтение
ограниченно сканирует корневые дескрипторы для доказательства имени владельца.
