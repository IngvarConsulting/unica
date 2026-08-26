---
id: DEC.2026-08-22.ATOMIC-CODE-REPLACEMENT-BATCH
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_replaces_multiple_anchors_atomically
changes: [CTR.WIRE.TOOL-SURFACE]
establishes: [INV.WIRE.CODE-PATCH-REPLACEMENT-BATCH, INV.SOURCE.CODE-PATCH-BATCH-ATOMICITY]
design: docs/design/2026-08-22-atomic-code-replacement-batch-design.md
---

# Code patch принимает атомарный пакет замен

**Решение.** `unica.code.patch` с `operation=replace` принимает либо прежнюю
плоскую замену, либо взаимоисключающий пакет из 1-50 закрытых элементов
`selector`, `content`, `expectedCount`. Все селекторы разрешаются на одном
исходном снимке, после чего единый постобраз проверяется и публикуется одной
мутацией.

Несовпадение кратности, пересечение диапазонов, невалидный постобраз, запрет
поддержки или конкурентное изменение завершают вызов без частичной записи.
Пакет остаётся границей одного BSL-модуля и не расширяет язык селекторов.
