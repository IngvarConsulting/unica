---
id: INV.WIRE.APPLY-REQUIRES-THE-FENCE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::two_plans_on_one_revision_cannot_both_publish
  - crates/unica-coder/src/application/v13/apply.rs::the_fence_is_required_by_the_mode_and_the_schema_says_so
---

# Применение требует ревизию и сохраняет уже опубликованную правку

`unica.apply` с `dryRun: false` или без `dryRun` требует `ifRev`.
Без него вызов отказывает до записи; опубликованная схема объявляет
это условие. Предпросмотр допускается как без `ifRev`, так и с ним.

Два плана на одной ревизии не могут оба опубликовать изменения:
после первого применения второй получает `stale_revision`, а правка
первого остаётся целой. Для нового применения нужна свежая ревизия.
