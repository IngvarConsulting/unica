---
id: DEC.2026-08-21.V2-PROCESS-POLICY
status: active
governs: process
realized: tests/arch/test_registry.py::test_v2_process_policy_changes_are_explicit_and_compatible
supersedes: []
superseded-by: null
establishes: [INV.DOC.PACKAGED-RELATIVE-LINKS, INV.DOC.PROJECT-NOTES-NON-NORMATIVE]
---

# Процессная политика v2 не переносит недоказанную широту v1

**Решение.** Нормативны записи только из `arch/`; `docs/design/` и
`docs/plans/` остаются происхождением и планом. Язык нормативной прозы не
ограничивается отдельным правилом. Единственность владельца обеспечивается
взаимной связью решения и правила, а ссылки активной поставляемой документации
разрешаются относительно содержащего файла.

**Почему.** Удалённый языковой checker и прежняя широкая проза об одном
владельце не должны изображаться перенесённым поведением узким regex-тестом.

**Цена.** Возврат языковой политики потребует нового процессного решения и
исполняемой проверки.
