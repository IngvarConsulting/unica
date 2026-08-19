---
id: INV.REGISTRY.NO-SELF-GLOSS
status: active
decision: DEC.2026-08-19.NO-SELF-GLOSS
check: tests/arch/test_registry.py::test_no_rule_explains_its_own_props
scope: [docs]
---

# Инвариант и контракт не толкуют свои поля

Тело записи не называет поле, которое эта же запись несёт в открывающем блоке.
Устройство реестра описано в `arch/README.md` и повторения не требует.
