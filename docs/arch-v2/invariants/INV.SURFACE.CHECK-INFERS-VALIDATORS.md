---
id: INV.SURFACE.CHECK-INFERS-VALIDATORS
status: active
governs: product
decision: DEC.2026-09-03.CHECK-INFERS-VALIDATORS
check: crates/unica-coder/src/application/v13/check.rs::every_node_kind_owns_its_validators_without_a_caller_choice
scope: [wire, app]
---

# Валидаторы узла следуют из его вида, а не из аргумента

`unica.check` над узлом запускает все валидаторы, которыми владеет вид узла,
и отдаёт объединённый вердикт со списком `validators`. На проводе у
`unica.check` нет аргумента, который выбирает валидатор; таблица вид → валидаторы закрыта
и проверяется целиком.
