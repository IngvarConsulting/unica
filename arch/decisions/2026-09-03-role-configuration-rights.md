---
id: DEC.2026-09-03.ROLE-CONFIGURATION-RIGHTS
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/v13_read/tests.rs::logical_reader_parity_contract_is_complete
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.LOGICAL-READER-PARITY]
design: docs/design/2026-09-03-role-configuration-rights-design.md
---

# Роль принимает права на конфигурацию в целом

**Решение.** Владелец права в Role — это kind из точного platform registry
`METADATA_KIND_TAGS` или корневой владелец `Configuration` с именем
конфигурации: объект `Configuration.<Имя>` несёт права Administration,
DataAdministration, UpdateDataBaseConfiguration, ExclusiveMode, ThinClient,
WebClient и остальные права конфигурации в целом. Такой объект проецируется
как `Role.<Роль>.Right.Configuration_<Имя>` с `objectKind = Configuration`;
короткий alias по имени конфигурации разрешается по тем же правилам, что и
у объектов метаданных. Ни один kind не содержит `_`, `kind_name` остаётся
инъективным; прочий type prefix по-прежнему даёт `provider_unavailable`.

**Почему.** Формат `Rights.xml` (`1c-role-spec.md`, «Права объектов верхнего
уровня → Configuration») задаёт права конфигурации объектом
`Configuration.<Имя>`; каждая административная роль УТ содержит его, и любой
`view` такой роли или `find` по конфигурации падал закрытым отказом.

**Цена.** Права конфигурации входят в индекс `find` как объект права роли;
их состав по-прежнему не валидируется по закрытому перечню.
