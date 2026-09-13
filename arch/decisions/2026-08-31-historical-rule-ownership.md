---
id: DEC.2026-08-31.HISTORICAL-RULE-OWNERSHIP
status: active
governs: process
realized: tests/arch/test_registry.py::test_current_rule_owner_establishes_the_rule
supersedes: []
superseded-by: null
establishes: [INV.REGISTRY.HISTORICAL-ESTABLISHES, INV.REGISTRY.UNBUILT-SUPERSESSION]
design: docs/design/2026-08-31-v0-13-surface-first-cutover-design.md
---

# Владение изменяемым правилом не переписывает историю решения

**Решение.** `decision.establishes` сохраняет исторический список правил,
выведенных решением. Изменяемое правило ссылается на текущего владельца, и этот
владелец обязан перечислять его; прежнее неизменяемое решение не удаляет свою
историческую ссылку при передаче владения.

Принятое, но не реализованное planned-решение может быть заменено с
`realized: null`: supersession доказывает отказ от направления, а не его
реализацию.

**Почему.** Обратная взаимность для всех исторических решений противоречила
запрету редактировать product decision и делала законное изменение правила
непредставимым. Требование evidence для непостроенного заменённого направления
смешивало реализацию с отказом от реализации.

**Цена.** Текущего владельца правила определяет только поле `rule.decision`;
исторический поиск происхождения учитывает все старые `establishes`.
