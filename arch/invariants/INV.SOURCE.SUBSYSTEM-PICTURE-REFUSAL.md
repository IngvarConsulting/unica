---
id: INV.SOURCE.SUBSYSTEM-PICTURE-REFUSAL
status: active
governs: product
decision: DEC.2026-09-13.SUBSYSTEM-PICTURE-PRESERVATION
check: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_subsystem_picture_property_refusal_is_write_free_in_both_modes
scope: [source, wire]
---

# Недопустимое свойство картинки отклоняется без записи в обоих режимах

`unica.apply` отклоняет ключи `LoadTransparent` и
`Picture.LoadTransparent` в `props.set` с одинаковой диагностикой и адресом
ошибки в preview и apply. Дерево рабочего пространства не меняется даже
при предшествующем `childSubsystem.add` в том же запросе.
