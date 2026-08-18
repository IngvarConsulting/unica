---
id: INV.DOC.ARCHIVE-FROZEN
status: active
decision: DEC.2026-08-18.ARCHITECTURE-RESET
check: tests/arch/test_registry.py::test_the_archive_is_not_edited_by_hand
scope: [docs]
---

# Архив v1 не изменяется после заморозки
`docs/arch-v1/**` не изменяется после заморозки. Запись архива отвечает на
вопрос, что было решено на её дату, и правка задним числом уничтожает
единственное, ради чего архив сохранён.
