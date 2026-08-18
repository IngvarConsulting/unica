---
id: INV.DOC.ARCHIVE-FROZEN
status: active
decision: DEC.2026-08-18.ARCHITECTURE-RESET
check: tests/arch/test_registry.py::test_archived_records_are_not_edited_after_the_freeze
scope: [docs]
---

# Замороженный слой не читается и не правится

`docs/arch-v1/**` не изменяется и не служит источником действующего правила. Его
собственный реестр говорит, что из него умерло, а что заменено — этого
достаточно; заходить внутрь за ответом о текущем поведении не нужно.
