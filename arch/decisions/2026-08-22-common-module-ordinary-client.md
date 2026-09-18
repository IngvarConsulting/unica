---
id: DEC.2026-08-22.COMMON-MODULE-ORDINARY-CLIENT
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_apply_view_round_trip_common_module_ordinary_client
changes: [CTR.WIRE.TOOL-SURFACE]
establishes: [INV.WIRE.COMMON-MODULE-ORDINARY-CLIENT]
design: docs/design/2026-08-22-common-module-ordinary-client-property-design.md
---

# Обычный клиент входит в свойства общего модуля

**Решение.** Операция `props.set` канонического `unica.apply` принимает
boolean-свойство `ClientOrdinaryApplication` только для `CommonModule`.
Реестр writer владеет типом, допустимым видом метаданных и записью Platform
XML. Предпросмотр не публикует изменения; применение с `ifRev` сохраняет
`true` и `false`. Неверный тип значения или вид объекта получает `bad_value`
без записи.

`unica.view` возвращает сохранённое значение в `props.commonModule.clientOrdinaryApplication`
по `CTR.SOURCE.MODULE-PROJECTION-SHAPE`. Профиль чтения остаётся независимым
от writer allowlist. Публичные имена определяет `CTR.WIRE.TOOL-SURFACE`.

Неизвестное свойство отклоняется с точным полем и перечнем поддерживаемых для
вида владельца альтернатив из того же реестра.
